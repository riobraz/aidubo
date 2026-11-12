//! Scheduled prompts.

use harness_protocol::{CreateScheduleRequest, Envelope, ScheduleInfo};
use reqwest::Method;

use crate::api::{SessionApi, decode_error, ensure_success, network_error};

impl SessionApi {
    pub async fn schedules(&self, session_id: &str) -> Result<Vec<ScheduleInfo>, String> {
        let response = self
            .request(Method::GET, "/api/experimental/schedule")
            .query(&[("sessionID", session_id)])
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<Vec<ScheduleInfo>> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    pub async fn create_schedule(
        &self,
        request: CreateScheduleRequest,
    ) -> Result<ScheduleInfo, String> {
        let response = self
            .request(Method::POST, "/api/experimental/schedule")
            .json(&request)
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<ScheduleInfo> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    /// Pause or resume; the server answers with the updated schedule.
    pub async fn schedule_action(&self, id: &str, action: &str) -> Result<ScheduleInfo, String> {
        let response = self
            .request(
                Method::POST,
                &format!("/api/experimental/schedule/{id}/{action}"),
            )
            .send()
            .await
            .map_err(network_error)?;
        let response = ensure_success(response).await?;
        let page: Envelope<ScheduleInfo> = response.json().await.map_err(decode_error)?;
        Ok(page.data)
    }

    pub async fn delete_schedule(&self, id: &str) -> Result<(), String> {
        let response = self
            .request(Method::DELETE, &format!("/api/experimental/schedule/{id}"))
            .send()
            .await
            .map_err(network_error)?;
        ensure_success(response).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_protocol::ScheduleWhen;

    #[test]
    fn schedule_create_body_shape() {
        let request = CreateScheduleRequest {
            session_id: "ses_1".into(),
            name: Some("later".into()),
            when: ScheduleWhen::in_("2 seconds"),
            if_condition: None,
            until: None,
            ends: Some(serde_json::json!({ "times": 1 })),
            prompt: "hello".into(),
        };
        let body = serde_json::to_value(&request).expect("body");
        assert_eq!(body["when"]["in"], "2 seconds");
        assert!(body.get("if").is_none());
        assert!(body.get("until").is_none());
        assert_eq!(body["ends"]["times"], 1);
    }
}
