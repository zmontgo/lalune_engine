use std::fmt;
use serde::Deserialize;
use std::collections::HashMap;
use chrono::{NaiveDate, NaiveDateTime};
use crate::errors;

/// Time periods for which to retrieve steps.
pub enum Period {
  OneDay,
  OneWeek,
  OneMonth,
  ThreeMonths,
  SixMonths,
  OneYear,
}

impl fmt::Display for Period {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match *self {
      Period::OneDay => write!(f, "1d"),
      Period::OneWeek => write!(f, "1w"),
      Period::OneMonth => write!(f, "1m"),
      Period::ThreeMonths => write!(f, "3m"),
      Period::SixMonths => write!(f, "6m"),
      Period::OneYear => write!(f, "1y"),
    }
  }
}

impl Period {
  pub fn to_str(&self) -> &str {
    match self {
      Period::OneDay => "1d",
      Period::OneWeek => "1w",
      Period::OneMonth => "1m",
      Period::ThreeMonths => "3m",
      Period::SixMonths => "6m",
      Period::OneYear => "1y",
    }
  }
}

#[derive(Debug, Deserialize)]
pub struct ErrorDetail {
  #[serde(rename = "errorType")]
  pub error_type: String,
  pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct ErrorResponse {
  pub errors: Vec<ErrorDetail>,
  #[serde(rename = "success")]
  _success: bool,
}

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
  pub access_token: String,
  pub expires_in: i32,
  pub refresh_token: String,
  pub scope: String,
  pub token_type: String,
  pub user_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum FitbitSuccess {
  Steps(HashMap<String, Vec<HashMap<String, String>>>),
  Refresh(TokenResponse),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum FitbitResponse {
  Success(FitbitSuccess),
  Error(ErrorResponse),
}

#[derive(Debug)]
pub struct Range {
  pub start: NaiveDate,
  pub end: NaiveDate,
}

#[derive(Debug)]
pub enum Command {
  GetSteps(String, Range),
  RefreshToken(String),
}

#[derive(Debug)]
pub enum Response {
  Steps(HashMap<NaiveDate, u32>),
  Refreshed,
  Error(errors::FitbitError),
}

#[derive(Debug)]
pub struct DatabaseUser {
  pub id: String,
  pub fitbit_user_id: String,
  pub fitbit_access_token: String,
  pub fitbit_refresh_token: String,
  pub fitbit_token_expires_at: NaiveDateTime,
}
