use std::io;

#[derive(Clone, Copy, Debug)]
pub enum PercentChange {
    Absolute(u8),
    Delta(i8),
    Multiply(u16),
    Divide(u16),
}

pub fn parse_percent_change(value: &str) -> io::Result<PercentChange> {
    let Some(value) = value.strip_suffix('%') else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "brightness value must end with %",
        ));
    };

    if let Some(delta) = value.strip_prefix('+') {
        return parse_delta(delta).map(PercentChange::Delta);
    }

    if let Some(delta) = value.strip_prefix('-') {
        return parse_delta(delta).map(|delta| PercentChange::Delta(-delta));
    }

    if let Some(factor) = value.strip_prefix('*') {
        return parse_factor(factor).map(PercentChange::Multiply);
    }

    if let Some(factor) = value.strip_prefix('/') {
        return parse_factor(factor).map(PercentChange::Divide);
    }

    let percent = value
        .parse::<u8>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;

    if percent > 100 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "brightness percent must be between 0 and 100",
        ));
    }

    Ok(PercentChange::Absolute(percent))
}

pub fn apply_percent_change(current: Option<u32>, change: PercentChange) -> u8 {
    match change {
        PercentChange::Absolute(percent) => percent,
        PercentChange::Delta(delta) => {
            let current = current.expect("relative percent change requires current value") as i16;
            current.saturating_add(i16::from(delta)).clamp(0, 100) as u8
        }
        PercentChange::Multiply(factor) => {
            let current = current.expect("relative percent change requires current value");
            let percent = ((current * u32::from(factor) + 50) / 100).clamp(0, 100);

            if percent == current && factor > 100 && current < 100 {
                (current + 1) as u8
            } else if percent == current && factor < 100 && current > 0 {
                (current - 1) as u8
            } else {
                percent as u8
            }
        }
        PercentChange::Divide(factor) => {
            let current = current.expect("relative percent change requires current value");
            let percent =
                ((current * 100 + u32::from(factor) / 2) / u32::from(factor)).clamp(0, 100);

            if percent == current && factor > 100 && current > 0 {
                (current - 1) as u8
            } else if percent == current && factor < 100 && current < 100 {
                (current + 1) as u8
            } else {
                percent as u8
            }
        }
    }
}

fn parse_delta(value: &str) -> io::Result<i8> {
    let delta = value
        .parse::<i8>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;

    if delta > 100 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "brightness delta must be between -100 and 100",
        ));
    }

    Ok(delta)
}

fn parse_factor(value: &str) -> io::Result<u16> {
    let factor = value
        .parse::<u16>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;

    if factor == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "brightness factor must be greater than 0%",
        ));
    }

    Ok(factor)
}
