use chrono::{NaiveDate, NaiveDateTime};
use std::collections::HashMap;
use std::convert::TryFrom;
use crate::models::{Command, Range, Response};
use crate::errors::FitbitError;
use ulid;
use log::info;

/// Parses a vector of tuples containing the date and the number of steps for that date into a vector of tuples containing the date and the number of steps for that date.
/// 
/// # Arguments
/// 
/// * `steps` - A vector of tuples containing the date and the number of steps for that date.
/// 
/// # Returns
/// 
/// * `Vec<(NaiveDate, u32)>` - A vector of tuples containing the date and the number of steps for that date.
pub fn parse_steps(steps: Vec<(u32, i64)>) -> Vec<(NaiveDate, u32)> {
  steps.into_iter().map(|(steps, date)| (NaiveDateTime::from_timestamp_opt(date, 0).unwrap().date(), steps)).collect()
}

/// Finds the longest range of consecutive dates in a vector of tuples containing the date and the number of steps for that date.
/// 
/// # Arguments
/// 
/// * `start_date` - The start date of the range.
/// * `steps` - A vector of tuples containing the date and the number of steps for that date.
/// 
/// # Returns
/// 
/// * `HashMap<NaiveDate, u32>` - A hashmap containing the date and the number of steps for that date.
pub fn longest_range(start_date: NaiveDate, steps: Vec<(NaiveDate, u32)>) -> HashMap<NaiveDate, u32> {
  let mut range: HashMap<NaiveDate, u32> = HashMap::new();
  let mut current_date = start_date;

  for (date, steps) in steps {
    if date == current_date {
      range.insert(date, steps);
    } else {
      break;
    }

    current_date = current_date.succ_opt().unwrap();
  }

  range    
}

/// Decodes a message from the Redis list into a command. The message is a vector of tuples containing the field and the value of the field.
/// 
/// # Arguments
/// 
/// * `message` - The message to decode, colon-separated, as such: `coordination_id:command:payload:TTL`.
///   * `coordination_id` - A ULID used to coordinate the command.
///   * `command` - The command to execute.
///   * `payload` - The payload of the command, colon-separated.
///   * `TTL` - The time-to-live of the command.
/// 
/// # Returns
/// 
/// * `Ok((coordination_id, Ok(task)))` - If the message was decoded successfully.
/// * `Ok((coordination_id, Err(e)))` - If the message was decoded successfully, but the command could not be parsed.
/// * `Err(e)` - If the message could not be decoded.
pub fn decode_message(message: String) -> Option<(ulid::Ulid, Result<Command, FitbitError>)> {
  let message_vector: Vec<&str> = message.split(":").collect();

  info!("Split message: {:?}", message_vector);

  let coordination_id = if !message_vector.is_empty() {
    match ulid::Ulid::from_string(message_vector[0]) {
      Ok(coordination_id) => coordination_id,
      Err(_) => {
        info!("Couldn't decode into ULID");
        return None
      },
    }
  } else {
    info!("Message vector was empty");
    return None;
  };

  if message_vector.len() != 4 {
    let message = format!("While decoding command, expected 4 fields: coordination_id, command, payload, TTL. Got {}", message);
    return Some((coordination_id, Err(FitbitError::InvalidMessage(message))));
  }

  info!("Command: {}  Payload: {}  TTL: {}", message_vector[1], message_vector[2], message_vector[3]);

  let command = message_vector[1];
  let payload = message_vector[2];
  let Some(ttl) = message_vector[3].parse::<i64>().ok() else {
    let ttl = message_vector[3];
    let message = format!("While decoding command, could not parse TTL to integer. Expected UNIX timestamp, got {}", ttl);
    return Some((coordination_id, Err(FitbitError::InvalidMessage(message))));
  };

  let Some(ttl) = NaiveDateTime::from_timestamp_opt(ttl, 0) else {
    let message = format!("While decoding command, could not parse TTL to NaiveDateTime. Expected UNIX timestamp, got {}", ttl);
    return Some((coordination_id, Err(FitbitError::InvalidMessage(message))));
  };

  if (ttl - chrono::Utc::now().naive_utc()).num_seconds() < 0 {
    return None;
  }

  match command {
    "get_steps" => {
      let parts: Vec<&str> = payload.split(",").collect();

      if parts.len() != 3 {
        let message = format!("While decoding get_steps command, expected user_id,start_timestamp,end_timestamp, got {}", payload);
        return Some((coordination_id, Err(FitbitError::InvalidMessage(message))));
      }

      let user_id = parts[0].to_string();

      let Ok(start_timestamp) = parts[1].parse::<i64>() else {
        let message = format!("While decoding get_steps command, could not parse start_timestamp to integer. Expected UNIX timestamp, got {}", parts[1]);
        return Some((coordination_id, Err(FitbitError::InvalidMessage(message))));
      };

      let Some(start) = NaiveDateTime::from_timestamp_opt(start_timestamp, 0) else {
        let message = format!("While decoding get_steps command, could not parse start_timestamp to NaiveDateTime. Expected UNIX timestamp, got {}", parts[1]);
        return Some((coordination_id, Err(FitbitError::InvalidMessage(message))));
      };

      let Ok(end_timestamp) = parts[2].parse::<i64>() else {
        let message = format!("While decoding get_steps command, could not parse end_timestamp to integer. Expected UNIX timestamp, got {}", parts[2]);
        return Some((coordination_id, Err(FitbitError::InvalidMessage(message))));
      };

      let Some(end) = NaiveDateTime::from_timestamp_opt(end_timestamp, 0) else {
        let message = format!("While decoding get_steps command, could not parse end_timestamp to NaiveDateTime. Expected UNIX timestamp, got {}", parts[2]);
        return Some((coordination_id, Err(FitbitError::InvalidMessage(message))));
      };

      let range = Range {
        start: start.date(),
        end: end.date(),
      };

      let command = Command::GetSteps(user_id, range);

      Some((coordination_id, Ok(command)))
    },
    "refresh" => {
      let parts = payload.split(",").collect::<Vec<&str>>();

      if parts.len() != 1 {
        let message = format!("While decoding refresh command, expected user_id, got {}", payload);
        return Some((coordination_id, Err(FitbitError::InvalidMessage(message))));
      }

      let user_id = parts[0].to_string();

      let command = Command::RefreshToken(user_id);

      Some((coordination_id, Ok(command)))
    },
    _ => Some((coordination_id, Err(FitbitError::InvalidMessage(format!("Unknown command, got {}", command))))),
  }
}

struct ListResponse {
  indication: String,
  content: String,
}

/// Encodes a response to be sent to the Redis list.
/// 
/// # Arguments
/// 
/// * `response` - The response to encode.
/// 
/// # Returns
/// 
/// * `Vec<String>` - The encoded response, as a string.
///     - The first element is the indication, which is either `0` for no error or `1` for an error.
///     - The second element is the content, which is either the response or the error message.
pub fn encode_response(response: Response) -> String {
  info!("Encoding response: {:?}", response);
  let response: ListResponse = match response {
    Response::Steps(steps) => {
      // Order the steps by date and convert to a vector of only the step count
      let mut steps = steps.into_iter().map(|(date, step_count)| {
        let Ok(step_count) = i32::try_from(step_count) else {
          return (date, 0);
        };

        (date, step_count)
      }).collect::<Vec<(NaiveDate, i32)>>();

      steps.sort_by(|a, b| a.0.cmp(&b.0));

      ListResponse {
        indication: String::from("0"),
        content: steps.into_iter().map(|(_, step_count)| format!("{step_count}")).collect::<Vec<String>>().join(","),
      }
    },
    Response::Refreshed => ListResponse {
      indication: String::from("0"),
      content: String::from("refreshed"),
    },
    Response::Error(error) => ListResponse {
      indication: String::from("1"),
      content: error.to_string(),
    },
  };

  // Escape the content
  let content = response.content.replace("\\", "\\\\")
    .replace(",", "\\,")
    .replace(":", "\\:")
    .replace("\n", "\\n");

  format!("{}:{}", response.indication, content)
}

/// Converts from i64 to T, clamping to the maximum value of T if the value is too large.
/// 
/// # Arguments
/// 
/// * `value` - The value to convert.
/// 
/// # Returns
/// 
/// * `T` - The converted value.
pub fn safe_convert<T: TryFrom<i64> + From<u16>>(value: i64) -> T {
  if value < 0 {
    T::from(0)
  } else if let Ok(v) = T::try_from(value) {
      v
  } else {
    T::from(u16::max_value())
  }
}