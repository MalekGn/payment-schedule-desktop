//! The typed error surface of every Tauri command.
//!
//! # Why this exists
//!
//! Commands used to return `Result<T, String>` built from
//! `map_err(|e| e.to_string())`, which sent the *entire* `rusqlite` error text
//! across the IPC boundary — constraint names, column names, SQL fragments,
//! filesystem paths. The frontend then rendered it verbatim in a toast, so a
//! shopkeeper could be shown `FOREIGN KEY constraint failed`, unlocalized.
//!
//! `AppError` splits errors into two populations:
//!
//! * **Actionable** (`Validation`, `Conflict`, `NotFound`) — the user did
//!   something the domain rejects, and there is a sensible localized sentence
//!   to show them. These carry a stable machine code.
//! * **Internal** — the user can do nothing about it. The detail is *logged*
//!   and the wire only ever sees the opaque code `INTERNAL`.
//!
//! # Wire format
//!
//! An error serializes to a plain string: `CODE` or `CODE:param[:param]`. That
//! shape predates this module (`CLIENT_HAS_PURCHASES:3`, `SUM_MISMATCH:900:1000`)
//! and is mirrored by `src/api/mock.ts` and asserted by the integration suite,
//! so keeping it means the mock, the tests and the frontend parser all stay
//! valid. `src/lib/errors.ts` maps a code to an `errors.*` i18n key.
//!
//! # The code inventory
//!
//! Every code below must have a matching key in **all three** of
//! `src/locales/{ar,fr,en}.json`.
//!
//! | Code                          | Meaning                                        |
//! | ----------------------------- | ---------------------------------------------- |
//! | `INVALID_DATE`                | A date field was not `YYYY-MM-DD`              |
//! | `INVALID_TOTAL_PRICE`         | Purchase total was not > 0                     |
//! | `INVALID_INSTALLMENT_COUNT`   | Installment count outside 1..=120              |
//! | `INVALID_INTERVAL_KIND`       | Interval kind not weekly/monthly/custom        |
//! | `INVALID_INTERVAL_DAYS`       | Custom interval outside 1..=365                |
//! | `INVALID_AMOUNT`              | Payment amount was not > 0                     |
//! | `SUM_MISMATCH:{sum}:{total}`  | Manual installments don't add up               |
//! | `OVERPAYMENT:{remaining}`     | Payment exceeds what the installment still owes|
//! | `CLIENT_HAS_PURCHASES:{n}`    | Unforced delete of a client that has purchases |
//! | `CLIENT_NOT_FOUND`            | No such client                                 |
//! | `PURCHASE_NOT_FOUND`          | No such purchase                               |
//! | `INSTALLMENT_NOT_FOUND`       | No such installment                            |
//! | `INVALID_LOGO_TYPE`           | Logo is not a supported image                  |
//! | `LOGO_TOO_LARGE`              | Logo exceeds the size cap                      |
//! | `BACKUP_FAILED`               | Database snapshot could not be written         |
//! | `INTERNAL`                    | Anything else; the detail is in the log only   |

use std::fmt;

/// An error that a command may return to the frontend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppError {
    /// The caller supplied a value the domain rejects.
    Validation(&'static str),
    /// The request conflicts with existing data. `detail` is appended after a
    /// colon so the frontend can interpolate it into the localized sentence.
    Conflict { code: &'static str, detail: String },
    /// The addressed row does not exist.
    NotFound(&'static str),
    /// Something the user cannot act on. Never serialized — see [`AppError::code`].
    Internal(String),
}

impl AppError {
    pub fn validation(code: &'static str) -> Self {
        AppError::Validation(code)
    }

    pub fn not_found(code: &'static str) -> Self {
        AppError::NotFound(code)
    }

    pub fn conflict(code: &'static str, detail: impl fmt::Display) -> Self {
        AppError::Conflict {
            code,
            detail: detail.to_string(),
        }
    }

    /// Build an internal error, logging the detail at `error` level as it is
    /// created.
    ///
    /// The logging lives here, in the constructor, rather than at each call
    /// site, because this is the one place guaranteed to see the detail before
    /// it is dropped: [`AppError::code`] deliberately throws it away so it can
    /// never reach the renderer. Every `From` impl funnels through here, so no
    /// database or IO failure can be silently swallowed.
    pub fn internal(detail: impl fmt::Display) -> Self {
        let detail = detail.to_string();
        log::error!("internal error: {detail}");
        AppError::Internal(detail)
    }

    /// The stable machine code sent to the frontend.
    ///
    /// `Internal` collapses to the opaque `INTERNAL`: this is the single point
    /// that stops SQL text, schema names and filesystem paths crossing IPC.
    pub fn code(&self) -> String {
        match self {
            AppError::Validation(code) | AppError::NotFound(code) => (*code).to_string(),
            AppError::Conflict { code, detail } if detail.is_empty() => (*code).to_string(),
            AppError::Conflict { code, detail } => format!("{code}:{detail}"),
            AppError::Internal(_) => "INTERNAL".to_string(),
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.code())
    }
}

impl std::error::Error for AppError {}

impl serde::Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.code())
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::internal(e)
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::internal(e)
    }
}

impl From<tauri::Error> for AppError {
    fn from(e: tauri::Error) -> Self {
        AppError::internal(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_detail_never_reaches_the_wire() {
        // The exact failure this type exists to prevent: a rusqlite message
        // naming a table/constraint must not be serializable to the renderer.
        let err = AppError::from(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(19),
            Some("FOREIGN KEY constraint failed on table client".into()),
        ));
        assert_eq!(err.code(), "INTERNAL");
        assert_eq!(serde_json::to_string(&err).unwrap(), "\"INTERNAL\"");
        // ...but it is still retained in-process for the log.
        assert!(matches!(err, AppError::Internal(d) if d.contains("FOREIGN KEY")));
    }

    #[test]
    fn actionable_codes_keep_their_params() {
        assert_eq!(
            AppError::validation("INVALID_AMOUNT").code(),
            "INVALID_AMOUNT"
        );
        assert_eq!(
            AppError::not_found("CLIENT_NOT_FOUND").code(),
            "CLIENT_NOT_FOUND"
        );
        assert_eq!(
            AppError::conflict("CLIENT_HAS_PURCHASES", 3).code(),
            "CLIENT_HAS_PURCHASES:3"
        );
        assert_eq!(
            AppError::conflict("SUM_MISMATCH", format_args!("{}:{}", 900, 1000)).code(),
            "SUM_MISMATCH:900:1000"
        );
        // An empty detail must not leave a dangling separator for the frontend
        // parser to split on.
        assert_eq!(AppError::conflict("OVERPAYMENT", "").code(), "OVERPAYMENT");
    }
}
