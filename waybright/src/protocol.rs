use std::io;

#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    List,
    Get { selector: String },
    Set { selector: String, change: String },
    Refresh,
}

pub fn parse_request(request: &str) -> io::Result<Request> {
    let mut parts = request.split_whitespace();
    let parsed = match parts.next() {
        Some("LIST") => Request::List,
        Some("GET") => Request::Get {
            selector: required(&mut parts, "output selector")?.to_owned(),
        },
        Some("SET") => Request::Set {
            selector: required(&mut parts, "output selector")?.to_owned(),
            change: required(&mut parts, "brightness change")?.to_owned(),
        },
        Some("REFRESH") => Request::Refresh,
        Some(command) => return Err(invalid(format!("unknown command {command}"))),
        None => return Err(invalid("empty request")),
    };

    if parts.next().is_some() {
        return Err(invalid("too many request arguments"));
    }
    Ok(parsed)
}

fn required<'a>(parts: &mut impl Iterator<Item = &'a str>, name: &str) -> io::Result<&'a str> {
    parts
        .next()
        .ok_or_else(|| invalid(format!("missing {name}")))
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_requests() {
        assert_eq!(parse_request("LIST").unwrap(), Request::List);
        assert_eq!(
            parse_request("SET @focused +5%").unwrap(),
            Request::Set {
                selector: "@focused".to_owned(),
                change: "+5%".to_owned(),
            }
        );
        assert!(parse_request("GET").is_err());
    }
}
