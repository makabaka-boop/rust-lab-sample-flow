use crate::error::{json_rejection, AppError};
use axum::async_trait;
use axum::body::Body;
use axum::extract::FromRequest;
use axum::http::Request;
use axum::Json;
use serde::de::DeserializeOwned;

pub struct ApiJson<T>(pub T);

#[async_trait]
impl<S, T> FromRequest<S, Body> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(request: Request<Body>, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(request, state)
            .await
            .map_err(json_rejection)?;
        Ok(ApiJson(value))
    }
}
