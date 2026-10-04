use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use maud::html;

use crate::views;

#[derive(Debug)]
pub enum AppError {
    NotFound(anyhow::Error),
    BadRequest(String),
    /// The store is empty; shown as a getting-started page.
    NoVersions,
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        Self::Internal(e)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, title, detail) = match &self {
            Self::NotFound(e) => (StatusCode::NOT_FOUND, "Not found", format!("{e:#}")),
            Self::BadRequest(msg) => (StatusCode::BAD_REQUEST, "Bad request", msg.clone()),
            Self::NoVersions => (
                StatusCode::OK,
                "No versions yet",
                "Import a client with `datamine import <client> --label <name>` to get started."
                    .to_owned(),
            ),
            Self::Internal(e) => {
                tracing::error!("{e:#}");
                // Details go to the log only; they can contain server paths.
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Something went wrong",
                    "The error has been logged.".to_owned(),
                )
            }
        };
        let body = views::page(
            None,
            title,
            views::Nav::None,
            html! {
                div.empty {
                    h1 { (title) }
                    p.muted { (detail) }
                    p { a.button href="/" { "Back to home" } }
                }
            },
        );
        (status, body).into_response()
    }
}
