#!/bin/bash
set -euo pipefail

# Package the macOS release build into a self-contained .app bundle and .dmg.
# Homebrew dylibs and runtime data (GStreamer plugins, gdk-pixbuf loaders,
# GSettings schemas, fontconfig config) are copied into the bundle and the
# install names rewritten so the app runs on any macOS machine.

APP_NAME="Yandex Music"
APP_BUNDLE="dist/${APP_NAME}.app"
MACOS_DIR="${APP_BUNDLE}/Contents/MacOS"
RES_DIR="${APP_BUNDLE}/Contents/Resources"
LIB_DIR="${RES_DIR}/lib"
SCHEMAS_DIR="${RES_DIR}/share/glib-2.0/schemas"
BIN_PATH="${MACOS_DIR}/yandex-music"

VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
BREW_PREFIX="$(brew --prefix)"

if [[ "$(uname)" != "Darwin" ]]; then
    echo "error: this script must run on macOS" >&2
    exit 1
fi

rm -rf "${APP_BUNDLE}"
mkdir -p "${MACOS_DIR}" "${LIB_DIR}" "${SCHEMAS_DIR}" "${RES_DIR}/etc"

cp target/release/yandex-music "${BIN_PATH}"
chmod +x "${BIN_PATH}"

# resolve_dep resolves a dependency string of a Mach-O file against the
# file's ORIGINAL location ("ref"), which is where @loader_path/@rpath are
# anchored. The resolved absolute path is printed on stdout.
resolve_dep() {
    local ref="$1" dep="$2" fdir
    fdir="$(dirname "${ref}")"
    case "${dep}" in
        @rpath/*)
            local base="${dep#@rpath/}"
            local rp
            while read -r rp; do
                local try
                case "${rp}" in
                    @loader_path/*) try="${fdir}/${rp#@loader_path/}" ;;
                    @executable_path/*) continue ;;
                    *) try="${rp}" ;;
                esac
                if [[ -f "${try}/${base}" ]]; then
                    echo "${try}/${base}"
                    return 0
                fi
            done < <(otool -l "${ref}" | awk '/LC_RPATH/{getline; getline; print $2}')
            return 1
            ;;
        @loader_path/*)
            local base="${dep#@loader_path/}"
            if [[ -f "${fdir}/${base}" ]]; then
                echo "${fdir}/${base}"
                return 0
            fi
            return 1
            ;;
        *)
            echo "${dep}"
            return 0
            ;;
    esac
}

# copy_deps copies every Homebrew dependency of a Mach-O file into the bundle
# and rewrites install names to @executable_path/../Resources/lib. "file" is
# the file to rewrite (already in the bundle), "ref" its original Homebrew
# path, used to resolve @loader_path/@rpath dependencies correctly.
copy_deps() {
    local file="$1" ref="$2"
    local skip=3
    if [[ -z "$(otool -D "${ref}" | tail -n +2)" ]]; then
        skip=2
    fi
    otool -L "${ref}" | tail -n +"${skip}" | awk '{print $1}' | while read -r dep; do
        local resolved
        if ! resolved="$(resolve_dep "${ref}" "${dep}")"; then
            echo "warning: could not resolve ${dep} (needed by $(basename "${file}"))" >&2
            continue
        fi
        case "${resolved}" in
            "${BREW_PREFIX}"/*)
                local base
                base="$(basename "${resolved}")"
                if [[ ! -f "${LIB_DIR}/${base}" ]]; then
                    cp "${resolved}" "${LIB_DIR}/${base}"
                    chmod +w "${LIB_DIR}/${base}"
                    install_name_tool -id "@executable_path/../Resources/lib/${base}" "${LIB_DIR}/${base}" 2>/dev/null || true
                    copy_deps "${LIB_DIR}/${base}" "${resolved}"
                fi
                install_name_tool -change "${dep}" "@executable_path/../Resources/lib/${base}" "${file}" 2>/dev/null || true
                ;;
        esac
    done
}

remove_brew_rpaths() {
    local file="$1"
    otool -l "${file}" | awk '/LC_RPATH/{getline; getline; print $2}' | while read -r rp; do
        case "${rp}" in
            "${BREW_PREFIX}"/*)
                install_name_tool -delete_rpath "${rp}" "${file}" 2>/dev/null || true
                ;;
        esac
    done
}

relocate() {
    local file="$1" ref="$2"
    chmod +w "${file}"
    copy_deps "${file}" "${ref}"
    remove_brew_rpaths "${file}"
    install_name_tool -add_rpath "@executable_path/../Resources/lib" "${file}" 2>/dev/null || true
}

echo "==> Bundling dylib dependencies"
relocate "${BIN_PATH}" "${BIN_PATH}"

# The gst-plugin-scanner helper must live next to the main binary so that the
# plugins' @executable_path-relative deps resolve to the bundle on any machine.
# Homebrew does not symlink it into libexec, so look it up in the Cellar.
echo "==> Bundling gst-plugin-scanner"
SCANNER_BIN="$(ls -1d "${BREW_PREFIX}"/Cellar/gstreamer/*/libexec/gstreamer-1.0/gst-plugin-scanner 2>/dev/null | sort -V | tail -1 || true)"
if [[ -n "${SCANNER_BIN}" && -x "${SCANNER_BIN}" ]]; then
    cp "${SCANNER_BIN}" "${MACOS_DIR}/gst-plugin-scanner"
    relocate "${MACOS_DIR}/gst-plugin-scanner" "${SCANNER_BIN}"
else
    echo "warning: gst-plugin-scanner not found; gstreamer will scan plugins in-process"
fi

copy_loose() {
    local dest="$1"
    shift
    for src in "$@"; do
        if [[ -e "${src}" ]]; then
            cp -L "${src}" "${dest}/"
        else
            echo "warning: skipping missing/broken ${src}"
        fi
    done
}

if [[ -d "${BREW_PREFIX}/lib/gstreamer-1.0" ]]; then
    echo "==> Bundling GStreamer plugins"
    mkdir -p "${LIB_DIR}/gstreamer-1.0"
    copy_loose "${LIB_DIR}/gstreamer-1.0" "${BREW_PREFIX}"/lib/gstreamer-1.0/*.dylib
    for plugin in "${LIB_DIR}"/gstreamer-1.0/*.dylib; do
        local_plugin="$(basename "${plugin}")"
        if [[ "${local_plugin}" == "libgstpython.dylib" ]]; then
            echo "warning: skipping gst-python plugin ${local_plugin} (unused, pulls in the Python framework)"
            rm -f "${plugin}"
            continue
        fi
        if otool -L "${BREW_PREFIX}/lib/gstreamer-1.0/${local_plugin}" | grep -qE 'libgtk-3\.0|libgdk-3\.0'; then
            echo "warning: skipping GTK3-dependent plugin ${local_plugin}"
            rm -f "${plugin}"
            continue
        fi
        relocate "${plugin}" "${BREW_PREFIX}/lib/gstreamer-1.0/${local_plugin}"
    done
fi

if [[ -d "${BREW_PREFIX}/lib/gdk-pixbuf-2.0" ]]; then
    echo "==> Bundling gdk-pixbuf loaders"
    PIXBUF_LOADERS_DIR="$(echo "${BREW_PREFIX}"/lib/gdk-pixbuf-2.0/*/loaders | awk '{print $1}')"
    mkdir -p "${LIB_DIR}/gdk-pixbuf-2.0/2.10.0/loaders"
    copy_loose "${LIB_DIR}/gdk-pixbuf-2.0/2.10.0/loaders" "${PIXBUF_LOADERS_DIR}"/*.so
    for loader in "${LIB_DIR}"/gdk-pixbuf-2.0/2.10.0/loaders/*.so; do
        relocate "${loader}" "${PIXBUF_LOADERS_DIR}/$(basename "${loader}")"
    done
    "${BREW_PREFIX}/bin/gdk-pixbuf-query-loaders" "${PIXBUF_LOADERS_DIR}"/*.so \
        > "${LIB_DIR}/gdk-pixbuf-2.0/2.10.0/loaders.cache"
    sed -i '' "s#${BREW_PREFIX}/lib/gdk-pixbuf-2.0#@executable_path/../Resources/lib/gdk-pixbuf-2.0#g" \
        "${LIB_DIR}/gdk-pixbuf-2.0/2.10.0/loaders.cache"
fi

echo "==> Bundling GSettings schemas"
copy_loose "${SCHEMAS_DIR}" "${BREW_PREFIX}"/share/glib-2.0/schemas/*.xml
"${BREW_PREFIX}/bin/glib-compile-schemas" "${SCHEMAS_DIR}"

echo "==> Bundling icon theme"
mkdir -p "${RES_DIR}/share/icons"
for theme in hicolor Adwaita; do
    if [[ -d "${BREW_PREFIX}/share/icons/${theme}" ]]; then
        cp -RL "${BREW_PREFIX}/share/icons/${theme}" "${RES_DIR}/share/icons/"
        rm -f "${RES_DIR}/share/icons/${theme}/icon-theme.cache"
        rm -rf "${RES_DIR}/share/icons/${theme}/cursors"
    fi
done

echo "==> Writing fontconfig config"
cat > "${RES_DIR}/etc/fonts.conf" <<'EOF'
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "fonts.dtd">
<fontconfig>
  <dir>/System/Library/Fonts</dir>
  <dir>/System/Library/Fonts/Supplemental</dir>
  <dir>/Library/Fonts</dir>
  <dir>~/.fonts</dir>
  <dir prefix="xdg">fonts</dir>
  <cachedir>~/.cache/fontconfig</cachedir>
  <match target="pattern">
    <edit name="dpi" mode="assign"><double>96</double></edit>
  </match>
</fontconfig>
EOF

echo "==> Writing Info.plist"
cat > "${APP_BUNDLE}/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleExecutable</key><string>yandex-music</string>
  <key>CFBundleIdentifier</key><string>dev.ymapp.yandex-music</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>Yandex Music</string>
  <key>CFBundleDisplayName</key><string>Yandex Music</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.music</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

echo "==> Ad-hoc code signing"
while IFS= read -r -d '' f; do
    if file -b "${f}" | grep -q '^Mach-O'; then
        codesign --force -s - "${f}" 2>/dev/null
    fi
done < <(find "${LIB_DIR}" -type f -print0)
codesign --force -s - "${BIN_PATH}"
if [[ -x "${MACOS_DIR}/gst-plugin-scanner" ]]; then
    codesign --force -s - "${MACOS_DIR}/gst-plugin-scanner"
fi
codesign --force -s - "${APP_BUNDLE}"

echo "==> Creating DMG"
hdiutil create \
    -volname "${APP_NAME}" \
    -srcfolder "${APP_BUNDLE}" \
    -ov \
    -format UDZO \
    "dist/Yandex-Music-${VERSION}.dmg"

ls -lh "dist/Yandex-Music-${VERSION}.dmg"
