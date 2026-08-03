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

copy_deps() {
    local file="$1"
    otool -L "${file}" | tail -n +2 | awk '{print $1}' | while read -r dep; do
        case "${dep}" in
            "${BREW_PREFIX}"/*)
                local base
                base="$(basename "${dep}")"
                if [[ ! -f "${LIB_DIR}/${base}" ]]; then
                    cp "${dep}" "${LIB_DIR}/${base}"
                    chmod +w "${LIB_DIR}/${base}"
                    install_name_tool -id "@executable_path/../Resources/lib/${base}" "${LIB_DIR}/${base}" 2>/dev/null || true
                    copy_deps "${LIB_DIR}/${base}"
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
    local file="$1"
    chmod +w "${file}"
    copy_deps "${file}"
    remove_brew_rpaths "${file}"
    install_name_tool -add_rpath "@executable_path/../Resources/lib" "${file}" 2>/dev/null || true
}

echo "==> Bundling dylib dependencies"
relocate "${BIN_PATH}"

if [[ -d "${BREW_PREFIX}/lib/gstreamer-1.0" ]]; then
    echo "==> Bundling GStreamer plugins"
    mkdir -p "${LIB_DIR}/gstreamer-1.0"
    cp "${BREW_PREFIX}"/lib/gstreamer-1.0/*.dylib "${LIB_DIR}/gstreamer-1.0/"
    for plugin in "${LIB_DIR}"/gstreamer-1.0/*.dylib; do
        relocate "${plugin}"
    done
fi

if [[ -d "${BREW_PREFIX}/lib/gdk-pixbuf-2.0" ]]; then
    echo "==> Bundling gdk-pixbuf loaders"
    PIXBUF_LOADERS_DIR="$(echo "${BREW_PREFIX}"/lib/gdk-pixbuf-2.0/*/loaders | awk '{print $1}')"
    mkdir -p "${LIB_DIR}/gdk-pixbuf-2.0/2.10.0/loaders"
    cp "${PIXBUF_LOADERS_DIR}"/*.so "${LIB_DIR}/gdk-pixbuf-2.0/2.10.0/loaders/"
    for loader in "${LIB_DIR}"/gdk-pixbuf-2.0/2.10.0/loaders/*.so; do
        relocate "${loader}"
    done
    "${BREW_PREFIX}/bin/gdk-pixbuf-query-loaders" "${LIB_DIR}"/gdk-pixbuf-2.0/2.10.0/loaders/*.so \
        > "${LIB_DIR}/gdk-pixbuf-2.0/2.10.0/loaders.cache"
    sed -i '' "s#${BREW_PREFIX}#@executable_path/../Resources/lib#g" \
        "${LIB_DIR}/gdk-pixbuf-2.0/2.10.0/loaders.cache"
fi

echo "==> Bundling GSettings schemas"
cp "${BREW_PREFIX}"/share/glib-2.0/schemas/*.xml "${SCHEMAS_DIR}/"
"${BREW_PREFIX}/bin/glib-compile-schemas" "${SCHEMAS_DIR}"

echo "==> Bundling icon theme"
if [[ -d "${BREW_PREFIX}/share/icons/hicolor" ]]; then
    mkdir -p "${RES_DIR}/share/icons"
    cp -R "${BREW_PREFIX}/share/icons/hicolor" "${RES_DIR}/share/icons/"
fi

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
codesign --force --deep -s - "${BIN_PATH}" 2>/dev/null || true
codesign --force --deep -s - "${APP_BUNDLE}" 2>/dev/null || true

echo "==> Creating DMG"
hdiutil create \
    -volname "${APP_NAME}" \
    -srcfolder "${APP_BUNDLE}" \
    -ov \
    -format UDZO \
    "dist/Yandex-Music-${VERSION}.dmg"

ls -lh "dist/Yandex-Music-${VERSION}.dmg"
