use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PercentChange {
    Absolute(u8),
    Delta(i16),
    Multiply(u32),
    Divide(u32),
}

pub fn parse_percent_change(value: &str) -> io::Result<PercentChange> {
    let Some(value) = value.strip_suffix('%') else {
        return Err(invalid("brightness must end with %"));
    };

    let change = match value.as_bytes().first() {
        Some(b'+') | Some(b'-') => PercentChange::Delta(
            value
                .parse()
                .map_err(|_| invalid("invalid brightness delta"))?,
        ),
        Some(b'*') => {
            let factor = parse_factor(&value[1..])?;
            if factor == 0 {
                return Err(invalid("brightness factor must not be zero"));
            }
            PercentChange::Multiply(factor)
        }
        Some(b'/') => {
            let factor = parse_factor(&value[1..])?;
            if factor == 0 {
                return Err(invalid("brightness divisor must not be zero"));
            }
            PercentChange::Divide(factor)
        }
        _ => {
            let percent = value
                .parse::<u8>()
                .map_err(|_| invalid("invalid brightness percentage"))?;
            if percent > 100 {
                return Err(invalid("brightness must be between 0 and 100"));
            }
            PercentChange::Absolute(percent)
        }
    };

    Ok(change)
}

pub fn apply_percent_change(current: u8, change: PercentChange) -> u8 {
    match change {
        PercentChange::Absolute(percent) => percent,
        PercentChange::Delta(delta) => (i16::from(current) + delta).clamp(0, 100) as u8,
        PercentChange::Multiply(factor) => rounded_scaled(current, factor, 100, factor.cmp(&100)),
        PercentChange::Divide(factor) => rounded_scaled(current, 100, factor, 100_u32.cmp(&factor)),
    }
}

pub fn validate_hardware_min(hardware_min: u8) -> io::Result<()> {
    if hardware_min == 0 || hardware_min >= 100 {
        return Err(invalid("hardware minimum must be between 1 and 99"));
    }
    Ok(())
}

pub fn effective_brightness(hardware: u32, software: u8, hardware_min: u8) -> u8 {
    let hardware_min = u32::from(hardware_min);
    if software < 100 {
        return ((u32::from(software) * hardware_min + 50) / 100).clamp(0, 100) as u8;
    }

    (hardware_min + (hardware.clamp(0, 100) * (100 - hardware_min) + 50) / 100).clamp(0, 100) as u8
}

pub fn split_brightness(percent: u8, hardware_min: u8) -> (u8, u8) {
    if percent >= hardware_min {
        let hardware = ((u32::from(percent - hardware_min) * 100
            + u32::from(100 - hardware_min) / 2)
            / u32::from(100 - hardware_min))
        .clamp(0, 100) as u8;
        return (hardware, 100);
    }

    let software = ((u32::from(percent) * 100 + u32::from(hardware_min) / 2)
        / u32::from(hardware_min))
    .clamp(0, 100) as u8;
    (0, software)
}

fn parse_factor(value: &str) -> io::Result<u32> {
    value
        .parse()
        .map_err(|_| invalid("invalid brightness factor"))
}

fn rounded_scaled(
    current: u8,
    numerator: u32,
    denominator: u32,
    direction: std::cmp::Ordering,
) -> u8 {
    let numerator = u64::from(numerator);
    let denominator = u64::from(denominator);
    let scaled = (u64::from(current) * numerator + denominator / 2) / denominator;
    let mut result = scaled.clamp(0, 100) as u8;
    if result == current {
        match direction {
            std::cmp::Ordering::Greater if current < 100 => result += 1,
            std::cmp::Ordering::Less if current > 0 => result -= 1,
            _ => {}
        }
    }
    result
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_applies_changes() {
        assert_eq!(
            parse_percent_change("50%").unwrap(),
            PercentChange::Absolute(50)
        );
        assert_eq!(
            apply_percent_change(50, parse_percent_change("+5%").unwrap()),
            55
        );
        assert_eq!(
            apply_percent_change(50, parse_percent_change("-60%").unwrap()),
            0
        );
        assert_eq!(
            apply_percent_change(10, parse_percent_change("*101%").unwrap()),
            11
        );
        assert_eq!(
            apply_percent_change(10, parse_percent_change("/101%").unwrap()),
            9
        );
        assert!(parse_percent_change("*0%").is_err());
        assert!(parse_percent_change("/0%").is_err());
    }

    #[test]
    fn combined_brightness_round_trips() {
        for percent in 0..=100 {
            let (hardware, software) = split_brightness(percent, 50);
            assert_eq!(
                effective_brightness(u32::from(hardware), software, 50),
                percent
            );
        }
    }
}
