use std::{
    env, io,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
};

#[derive(Clone, Debug)]
pub struct OutputState {
    pub name: String,
    pub brightness: u8,
    pub hardware: Option<u32>,
    pub software: Option<u8>,
}

pub fn list() -> io::Result<Vec<OutputState>> {
    send("LIST")?.lines().map(parse_output).collect()
}

pub fn get(selector: &str) -> io::Result<u8> {
    validate_argument(selector)?;
    send(&format!("GET {selector}"))?
        .trim()
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub fn set(selector: &str, change: &str) -> io::Result<()> {
    validate_argument(selector)?;
    validate_argument(change)?;
    expect_ok(send(&format!("SET {selector} {change}"))?)
}

pub fn refresh() -> io::Result<()> {
    expect_ok(send("REFRESH")?)
}

pub fn socket_path() -> io::Result<PathBuf> {
    if let Some(socket_path) = env::var_os("WAYBRIGHT_SOCKET") {
        return Ok(PathBuf::from(socket_path));
    }
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))?;
    Ok(runtime_dir.join("waybright.sock"))
}

fn send(request: &str) -> io::Result<String> {
    let mut stream = UnixStream::connect(socket_path()?)?;
    stream.write_all(request.as_bytes())?;
    stream.shutdown(std::net::Shutdown::Write)?;

    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    if let Some(error) = response.trim().strip_prefix("ERR ") {
        return Err(io::Error::other(error.to_owned()));
    }
    Ok(response)
}

fn parse_output(line: &str) -> io::Result<OutputState> {
    let mut fields = line.split('\t');
    Ok(OutputState {
        name: required(&mut fields, "output name")?.to_owned(),
        brightness: parse(required(&mut fields, "brightness")?)?,
        hardware: parse_optional(required(&mut fields, "hardware brightness")?)?,
        software: parse_optional(required(&mut fields, "software brightness")?)?,
    })
}

fn required<'a>(fields: &mut impl Iterator<Item = &'a str>, name: &str) -> io::Result<&'a str> {
    fields
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("missing {name}")))
}

fn parse<T: std::str::FromStr>(value: &str) -> io::Result<T>
where
    T::Err: std::error::Error + Send + Sync + 'static,
{
    value
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn parse_optional<T: std::str::FromStr>(value: &str) -> io::Result<Option<T>>
where
    T::Err: std::error::Error + Send + Sync + 'static,
{
    match value {
        "-" => Ok(None),
        value => parse(value).map(Some),
    }
}

fn expect_ok(response: String) -> io::Result<()> {
    if response.trim() == "OK" {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid daemon response",
        ))
    }
}

fn validate_argument(value: &str) -> io::Result<()> {
    if value.is_empty() || value.chars().any(char::is_whitespace) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "request arguments must not contain whitespace",
        ));
    }
    Ok(())
}
