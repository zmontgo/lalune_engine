use std::collections::HashMap;
use chrono::NaiveDate;
use reqwest::header::HeaderMap;
use base64::{Engine as _, engine::general_purpose};
use crate::models::{Period, FitbitResponse, FitbitSuccess, TokenResponse};
use crate::errors::FitbitError;
use log::{error, info};

/// Get steps for a given end date and period. All dates are UTC.
/// 
/// # Arguments
/// 
/// * `date` - The end date for which to retrieve steps.
/// * `period` - The period for which to retrieve steps.
/// 
/// # Examples
/// 
/// This example gets the steps for the week ending on January 1, 2023.
/// 
/// ```
/// use chrono::NaiveDate;
/// use fitbit_steps::Period;
/// 
/// #[tokio::main]
/// async fn main() {
///   let date = NaiveDate::from_ymd_opt(2023, 1, 1).unwrap();
///   let period = Period::OneWeek;
/// 
///   let steps = fitbit_steps::get_steps(date, period).await.unwrap();
///   println!("{:?}", steps);
/// }
/// ```
/// 
/// # Errors
/// 
/// Returns an error if the request fails or if the response is malformed.
pub async fn get_steps(client: &reqwest::Client, user_id: &str, access_token: &str, date: NaiveDate, period: Period) -> Result<(HashMap<NaiveDate, u32>, HeaderMap), FitbitError> {
  // Test
  let test_url = format!("https://api.fitbit.com/1/user/{}/profile.json", user_id);
  let test_auth = format!("Bearer {}", access_token);

  let test_resp = client.get(test_url)
    .header("Authorization", test_auth)
    .send()
    .await;

  let Ok(test_resp) = test_resp else {
    error!("Couldn't get profile");
    return Err(FitbitError::HttpRequestError(test_resp.unwrap_err()));
  };

  info!("{:?}", test_resp.text().await);



  let date = date.format("%Y-%m-%d").to_string();
  let url: String = format!("https://api.fitbit.com/1/user/{}/activities/steps/date/{}/{}.json?timezone=UTC", user_id, date, period.to_str());
  let auth: String = format!("Bearer {}", access_token);

  let resp = client.get(url)
    .header("Authorization", auth)
    .send()
    .await;

  let resp = match resp {
    Ok(resp) => resp,
    Err(e) => return Err(FitbitError::HttpRequestError(e)),
  };

  let headers = resp.headers().clone();

  let resp = resp
    .json::<FitbitResponse>()
    .await;

  let resp = match resp {
    Ok(FitbitResponse::Success(FitbitSuccess::Steps(steps))) => steps,
    Ok(FitbitResponse::Error(e)) => {
      if let Some(error_detail) = e.errors.get(0) {
        if error_detail.error_type == "expired_token" {
          return Err(FitbitError::ExpiredToken);
        }

        return Err(FitbitError::FitbitApiError(error_detail.message.clone()));
      }

      return Err(FitbitError::ParsingError("Empty error list".to_string()));
    },
    Err(e) => return Err(FitbitError::ParsingError(e.to_string())),
    _ => return Err(FitbitError::ParsingError("Failed to parse response".to_string())),
  };

  if !resp.contains_key("activities-steps") || resp["activities-steps"].is_empty() {
    return Err(FitbitError::ParsingError("No steps found".to_string()));
  }

  let steps = parse_steps(&resp["activities-steps"]);

  match steps {
    Ok(steps) => Ok((steps, headers)),
    Err(e) => Err(FitbitError::ParsingError(e.to_string())),
  }
}

fn parse_steps(steps: &Vec<HashMap<String, String>>) -> Result<HashMap<NaiveDate, u32>, Box<dyn std::error::Error>> {
  let mut parsed_steps: HashMap<NaiveDate, u32> = HashMap::new();

  for step in steps {
    let date = NaiveDate::parse_from_str(&step["dateTime"], "%Y-%m-%d")
      .map_err(|_| "Failed to parse date")?;
    let value = step["value"].parse::<u32>()
      .map_err(|_| "Failed to parse value")?;

    parsed_steps.insert(date, value);
  }

  Ok(parsed_steps)
}

pub async fn refresh_token(client: &reqwest::Client, refresh_token: &str, client_id: &str, client_secret: &str) -> Result<TokenResponse, FitbitError> {
  let authorization = general_purpose::STANDARD_NO_PAD.encode(format!("{}:{}", client_id, client_secret).as_bytes());
  let resp = client.post("https://api.fitbit.com/oauth2/token")
    .form(&[
      ("grant_type", "refresh_token"),
      ("refresh_token", refresh_token),
    ])
    .header("authorization", format!("Basic {}", authorization))
    .send()
    .await;

  let resp = match resp {
    Ok(resp) => resp,
    Err(e) => return Err(FitbitError::HttpRequestError(e)),
  };

  let resp = resp
    .json::<FitbitResponse>()
    .await;

  let resp = match resp {
    Ok(FitbitResponse::Success(FitbitSuccess::Refresh(data))) => data,
    Ok(FitbitResponse::Error(e)) => {
      if let Some(error_detail) = e.errors.get(0) {
        return Err(FitbitError::FitbitApiError(error_detail.message.clone()));
      }

      return Err(FitbitError::ParsingError("Empty error list".to_string()));
    },
    Err(e) => return Err(FitbitError::ParsingError(e.to_string())),
    _ => return Err(FitbitError::ParsingError("Failed to parse response".to_string())),
  };

  let data: TokenResponse = TokenResponse { access_token: resp.access_token, expires_in: resp.expires_in, refresh_token: resp.refresh_token, scope: resp.scope, token_type: resp.token_type, user_id: resp.user_id };

  Ok(data)
}