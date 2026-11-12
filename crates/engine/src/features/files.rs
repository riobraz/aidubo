//! Project file search for the `@` picker.

use harness_protocol::{Envelope, FileEntry};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    /// Search files under `directory` for the `@` picker.
    pub async fn find_files(
        &self,
        directory: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<FileEntry>, String> {
        let limit = limit.to_string();
        let response = self
            .request(Method::GET, "/api/fs/find")
            .query(&[
                ("location[directory]", directory),
                ("query", query),
                ("type", "file"),
                ("limit", limit.as_str()),
            ])
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let list: Envelope<Vec<FileEntry>> = response.json().await.map_err(decode_error)?;
        Ok(list.data)
    }
}
