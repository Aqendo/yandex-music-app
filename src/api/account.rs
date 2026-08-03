//! Account endpoints.
use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::api::{ApiClient, ApiError, Status};

impl ApiClient {
    /// Fetch the account status (mirrors the reference client's `accountStatus`).
    ///
    /// Also records the account uid for later `/users/{uid}/...` calls.
    ///
    /// Right after a device-flow authorization the `/account/status` payload can
    /// briefly omit `uid` while the account record propagates; retry briefly.
    pub async fn account_status(&self) -> Result<Status, ApiError> {
        for _ in 0..3 {
            let status: Status = self.get("/account/status", &[]).await?;
            if status.account.uid.is_some() {
                self.set_uid(status.account.uid);
                return Ok(status);
            }
            tokio::time::sleep(Duration::from_millis(1500)).await;
        }
        let status: Status = self.get("/account/status", &[]).await?;
        self.set_uid(status.account.uid);
        Ok(status)
    }

    /// Batch-fetch entities of a given type, like the reference `getList`.
    pub(crate) async fn get_list<T: DeserializeOwned>(
        &self,
        object_type: &str,
        ids: &[crate::api::Id],
        params: &[(&str, String)],
    ) -> Result<Vec<T>, ApiError> {
        let id_key = format!("{object_type}-ids");
        let mut form = crate::api::ids_form(&id_key, ids);
        form.extend_from_slice(params);
        self.post_form(&format!("/{object_type}s"), &form).await
    }
}
