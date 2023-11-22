use std::{collections::HashMap, path::Path};

use fpush_traits::push::{PushError, PushResult, PushTrait};

use async_trait::async_trait;
use google_fcm1::{
    api::{Message, SendMessageRequest, Notification, AndroidConfig, AndroidNotification, ApnsConfig},
    oauth2, FirebaseCloudMessaging,
};
use log::{error, warn};

use serde::Deserialize;

use crate::config::GoogleFcmConfig;
pub struct FpushFcm {
    fcm_conn:
        FirebaseCloudMessaging<hyper_rustls::HttpsConnector<hyper::client::connect::HttpConnector>>,
    fcm_parent: String,
}

impl FpushFcm {
    async fn load_oauth2_app_secret(fcm_config: &GoogleFcmConfig) -> oauth2::ServiceAccountKey {
        match oauth2::read_service_account_key(Path::new(fcm_config.fcm_secret_path())).await {
            Ok(s) => s,
            Err(e) => panic!(
                "Could not read fcm config file at {} reason: {}",
                fcm_config.fcm_secret_path(),
                e
            ),
        }
    }

    pub async fn init(fcm_config: &GoogleFcmConfig) -> PushResult<Self> {
        let fcm_secret = Self::load_oauth2_app_secret(fcm_config).await;

        // create login auth object
        let auth = match oauth2::ServiceAccountAuthenticator::builder(fcm_secret.clone())
            .build()
            .await
        {
            Ok(auth) => auth,
            Err(e) => {
                error!("Could not load fcm DeviceFlowAuthenticator: {}", e);
                return Err(PushError::CertLoading);
            }
        };

        let hyper_client = hyper::Client::builder().build(
            hyper_rustls::HttpsConnectorBuilder::new()
                .with_native_roots()
                .https_only()
                .enable_http2()
                .build(),
        );
        let fcm_conn = FirebaseCloudMessaging::new(hyper_client, auth);
        Ok(Self {
            fcm_conn,
            fcm_parent: format!("projects/{}", fcm_secret.project_id.unwrap()),
        })
    }
}

#[derive(Debug, Deserialize)]
struct FcmError {
    error_code: FcmErrorCode,
}

impl FcmError {
    pub fn error_code(&self) -> &FcmErrorCode {
        &self.error_code
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum FcmErrorCode {
    UnspecifiedError,
    InvalidArgument,
    Unregistered,
    SenderIdMismatch,
    QuotaExceeded,
    Unavailable,
    Internal,
    ThirdPartyAuthError,
}

#[async_trait]
impl PushTrait for FpushFcm {
    #[inline(always)]
    async fn send(&self, token: String) -> PushResult<()> {
        let req = SendMessageRequest {
            message: Some(create_push_message(token)),
            validate_only: None,
        };

        let fcm_result = self
            .fcm_conn
            .projects()
            .messages_send(req, &self.fcm_parent)
            .doit()
            .await;
        match fcm_result {
            Err(e) => {
                warn!("FCM returned {}", e);
                if let google_fcm1::client::Error::BadRequest(error_body) = e {
                    let parsed_error_body: FcmError = serde_json::from_value(error_body).unwrap();
                    match parsed_error_body.error_code() {
                        FcmErrorCode::Unregistered => Err(PushError::TokenBlocked),
                        FcmErrorCode::QuotaExceeded => Err(PushError::TokenRateLimited),
                        FcmErrorCode::Unavailable => Err(PushError::PushEndpointTmp),
                        FcmErrorCode::Internal => Err(PushError::PushEndpointTmp),
                        FcmErrorCode::SenderIdMismatch => Err(PushError::TokenBlocked),
                        _ => Err(PushError::Unkown(u16::MAX)),
                    }
                } else {
                    Err(PushError::PushEndpointTmp)
                }
            }
            Ok(_) => Ok(()),
        }
    }
}

#[inline(always)]
fn create_push_message(token: String) -> Message {
    Message {
        data: Some(HashMap::new()),
        token: Some(token),
        notification: Some(create_notification()),
        // add this to make sure we set the tag to group on Android
        // so the user only ever sees 1 notification in their drawer
        android: Some(create_android_config()),
        apns: Some(create_apns_config()),
        ..Default::default()
    }
}

#[inline(always)]
fn create_notification() -> Notification {
    Notification {
        body: Some("You have new messages".to_string()),
        title: Some("Fedi Alpha".to_string()),
        ..Default::default()
    }
}


#[inline(always)]
fn create_android_config() -> AndroidConfig {
    AndroidConfig {
        notification: Some(create_android_notification()),
        ..Default::default()
    }
}

#[inline(always)]
fn create_android_notification() -> AndroidNotification {
    AndroidNotification {
        tag: Some("new_chat_messages".to_string()),
        ..Default::default()
    }
}

#[inline(always)]
fn create_apns_config() -> ApnsConfig {
    ApnsConfig {
        headers: Some(create_ios_notification()),
        ..Default::default()
    }
}

#[inline(always)]
fn create_ios_notification() -> HashMap<String, String> {
    let mut map = HashMap::new();
    map.insert("apns-collapse-id".to_string(), "new_chat_messages".to_string());
    map
}