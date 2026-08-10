//! Error types mirroring `PurchasesError` / `PurchasesErrorCode` from the
//! official RevenueCat SDKs.

use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

/// Mirrors the cross-platform `PurchasesErrorCode` enum shared by
/// purchases-ios / purchases-android / purchases-js.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[non_exhaustive]
pub enum ErrorCode {
    UnknownError,
    PurchaseCancelledError,
    StoreProblemError,
    PurchaseNotAllowedError,
    PurchaseInvalidError,
    ProductNotAvailableForPurchaseError,
    ProductAlreadyPurchasedError,
    ReceiptAlreadyInUseError,
    InvalidReceiptError,
    MissingReceiptFileError,
    NetworkError,
    InvalidCredentialsError,
    UnexpectedBackendResponseError,
    InvalidAppUserIdError,
    OperationAlreadyInProgressError,
    UnknownBackendError,
    IneligibleError,
    InsufficientPermissionsError,
    PaymentPendingError,
    InvalidSubscriberAttributesError,
    LogOutWithAnonymousUserError,
    ConfigurationError,
    UnsupportedError,
    EmptySubscriberAttributesError,
    CustomerInfoError,
    SignatureVerificationError,
    InvalidEmailError,
}

impl ErrorCode {
    pub fn description(&self) -> &'static str {
        match self {
            Self::UnknownError => "Unknown error.",
            Self::PurchaseCancelledError => "Purchase was cancelled.",
            Self::StoreProblemError => "There was a problem with the store.",
            Self::PurchaseNotAllowedError => {
                "The device or user is not allowed to make the purchase."
            }
            Self::PurchaseInvalidError => "One or more of the arguments provided are invalid.",
            Self::ProductNotAvailableForPurchaseError => {
                "The product is not available for purchase."
            }
            Self::ProductAlreadyPurchasedError => "This product is already active for the user.",
            Self::ReceiptAlreadyInUseError => {
                "The receipt is already in use by another subscriber."
            }
            Self::InvalidReceiptError => "The receipt is not valid.",
            Self::MissingReceiptFileError => "The receipt is missing.",
            Self::NetworkError => "Error performing request.",
            Self::InvalidCredentialsError => {
                "There was a credentials issue. Check the underlying error for more details."
            }
            Self::UnexpectedBackendResponseError => {
                "Received unexpected response from the backend."
            }
            Self::InvalidAppUserIdError => "The app user id is not valid.",
            Self::OperationAlreadyInProgressError => "The operation is already in progress.",
            Self::UnknownBackendError => "There was an unknown backend error.",
            Self::IneligibleError => "The User is ineligible for that action.",
            Self::InsufficientPermissionsError => {
                "App does not have sufficient permissions to make purchases."
            }
            Self::PaymentPendingError => "The payment is pending.",
            Self::InvalidSubscriberAttributesError => {
                "One or more of the attributes sent could not be saved."
            }
            Self::LogOutWithAnonymousUserError => {
                "Called logOut but the current user is anonymous."
            }
            Self::ConfigurationError => "There is an issue with your configuration.",
            Self::UnsupportedError => {
                "There was a problem with the operation. This is not supported."
            }
            Self::EmptySubscriberAttributesError => "Attributes are empty.",
            Self::CustomerInfoError => "There was a problem related to the customer info.",
            Self::SignatureVerificationError => {
                "Request or response signature verification failed."
            }
            Self::InvalidEmailError => "Email is not valid.",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

/// Mirrors `PurchasesError`: a stable code plus human-readable context.
#[derive(Debug, Clone, thiserror::Error, serde::Serialize)]
#[error("{code}: {message}")]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
    /// Underlying cause (backend error message, transport error, ...).
    pub underlying: Option<String>,
    /// Numeric backend error code (`{"code": 7259, ...}`) when the error
    /// originated from the RevenueCat API.
    pub backend_code: Option<i64>,
    /// The full parsed error body, for flows that read extra fields (e.g.
    /// `purchase_redemption_error_info.obfuscated_email`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_body: Option<serde_json::Value>,
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            underlying: None,
            backend_code: None,
            error_body: None,
        }
    }

    pub fn with_underlying(code: ErrorCode, underlying: impl Into<String>) -> Self {
        Self {
            code,
            message: code.description().to_owned(),
            underlying: Some(underlying.into()),
            backend_code: None,
            error_body: None,
        }
    }

    pub fn from_backend(backend_code: i64, backend_message: impl Into<String>) -> Self {
        Self {
            code: map_backend_code(backend_code),
            message: backend_message.into(),
            underlying: None,
            backend_code: Some(backend_code),
            error_body: None,
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Self::with_underlying(ErrorCode::NetworkError, err.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Self::with_underlying(ErrorCode::UnexpectedBackendResponseError, err.to_string())
    }
}

/// Maps RevenueCat backend error codes (`{"code": 7xxx}`) to SDK error codes.
/// Table taken from the generated cross-SDK mapping shared by purchases-js
/// (`src/networking/errors.ts`), purchases-android, and purchases-ios.
pub(crate) fn map_backend_code(code: i64) -> ErrorCode {
    match code {
        7000 => ErrorCode::ConfigurationError, // invalid platform
        7012 | 7834 | 7879 => ErrorCode::InvalidEmailError,
        7101 | 7229 | 7231 | 7773 => ErrorCode::StoreProblemError,
        7898..=7901 => ErrorCode::StoreProblemError, // gateway setup errors
        7102 => ErrorCode::ReceiptAlreadyInUseError, // cannot transfer purchase
        7103 => ErrorCode::InvalidReceiptError,      // invalid receipt token
        7104 | 7110 | 7226 | 7234 => ErrorCode::UnexpectedBackendResponseError,
        7105 | 7106 | 7814 | 7849 | 7853 | 7877 | 7878 => ErrorCode::PurchaseInvalidError,
        7107 | 7224 | 7225 | 7967 => ErrorCode::InvalidCredentialsError,
        7220 | 7256 => ErrorCode::InvalidAppUserIdError, // empty / invalid app user id
        7230 | 7255 => ErrorCode::ConfigurationError,    // invalid package name / alias
        7232 => ErrorCode::IneligibleError,              // ineligible for promo offer
        7259 => ErrorCode::CustomerInfoError,            // subscriber not found
        7263 | 7264 => ErrorCode::InvalidSubscriberAttributesError,
        7629 | 7638 => ErrorCode::OperationAlreadyInProgressError,
        7651 => ErrorCode::PaymentPendingError, // payment not complete
        7662 => ErrorCode::UnsupportedError,    // product ids malformed
        7772 | 7852 => ErrorCode::ProductAlreadyPurchasedError,
        _ => ErrorCode::UnknownBackendError,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_backend_codes() {
        assert_eq!(map_backend_code(7102), ErrorCode::ReceiptAlreadyInUseError);
        assert_eq!(map_backend_code(7225), ErrorCode::InvalidCredentialsError);
        assert_eq!(map_backend_code(7259), ErrorCode::CustomerInfoError);
        assert_eq!(
            map_backend_code(7772),
            ErrorCode::ProductAlreadyPurchasedError
        );
        assert_eq!(map_backend_code(9999), ErrorCode::UnknownBackendError);
    }

    #[test]
    fn backend_error_carries_code_and_message() {
        let error = Error::from_backend(7263, "Some attributes could not be saved.");
        assert_eq!(error.code, ErrorCode::InvalidSubscriberAttributesError);
        assert_eq!(error.backend_code, Some(7263));
    }
}
