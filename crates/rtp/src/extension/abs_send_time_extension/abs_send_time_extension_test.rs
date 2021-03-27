#[cfg(test)]
mod tests {
    use chrono::FixedOffset;
    use chrono::TimeZone;
    use std::ops::Sub;
    use std::time::Duration;

    use crate::{
        errors::ExtensionError,
        extension::abs_send_time_extension::{ntp2unix, unix2ntp, AbsSendTimeExtension},
    };

    const ABS_SEND_TIME_RESOLUTION: i128 = 1000;

    #[test]
    fn test_ntp_conversion() {
        let loc = FixedOffset::west(5 * 60 * 60); // UTC-5
        let tests = vec![
            (
                loc.ymd(1985, 6, 23).and_hms_nano(4, 0, 0, 0),
                0xa0c65b1000000000 as u64,
            ),
            (
                loc.ymd(1999, 12, 31).and_hms_nano(23, 59, 59, 500000),
                0xbc18084f0020c49b as u64,
            ),
            (
                loc.ymd(2019, 3, 27).and_hms_nano(13, 39, 30, 8675309),
                0xe04641e202388b88 as u64,
            ),
        ];

        for (t, n) in &tests {
            let ntp = unix2ntp(Duration::from_nanos(t.timestamp_nanos() as u64));
            assert_eq!(ntp, *n, "unix2ntp error");
        }

        for (t, n) in &tests {
            let output = ntp2unix(*n);
            let input = Duration::from_nanos(t.timestamp_nanos() as u64);
            let diff = input.sub(output).as_nanos() as i128;
            if diff < -ABS_SEND_TIME_RESOLUTION || ABS_SEND_TIME_RESOLUTION < diff {
                panic!(
                    "Converted time.Time from NTP time differs, expected: {:?}, got: {:?}",
                    input, output,
                );
            }
        }
    }

    #[test]
    fn test_abs_send_time_extension_roundtrip() -> Result<(), ExtensionError> {
        let tests = vec![
            AbsSendTimeExtension { timestamp: 123456 },
            AbsSendTimeExtension { timestamp: 654321 },
        ];

        for (i, test) in tests.iter().enumerate() {
            let mut b = test.marshal()?;

            let mut out = AbsSendTimeExtension::default();

            out.unmarshal(&mut b)?;
            assert_eq!(
                test.timestamp, out.timestamp,
                "[{}] Timestamp differs, expected: {}, got: {}",
                i, test.timestamp, out.timestamp
            );
        }

        Ok(())
    }

    #[test]
    fn test_abs_send_time_extension_estimate() -> Result<(), ExtensionError> {
        let tests = vec![
            //FFFFFFC000000000 mask of second
            (0xa0c65b1000100000, 0xa0c65b1001000000), // not carried
            (0xa0c65b3f00000000, 0xa0c65b4001000000), // carried during transmission
        ];

        for (i, (send_ntp, receive_ntp)) in tests.iter().enumerate() {
            let in_time = ntp2unix(*send_ntp);
            let send = AbsSendTimeExtension {
                timestamp: send_ntp >> 14,
            };

            let mut b = send.marshal()?;

            let mut received = AbsSendTimeExtension::default();
            received.unmarshal(&mut b)?;

            let estimated = received.estimate(ntp2unix(*receive_ntp));
            let diff = estimated.sub(in_time).as_nanos() as i128;
            if diff < -ABS_SEND_TIME_RESOLUTION || ABS_SEND_TIME_RESOLUTION < diff {
                panic!(
                    "[{}] Estimated time differs, expected: {:?}, estimated: {:?} (receive time: {})",
                    i, in_time, estimated,receive_ntp,
                );
            }
        }

        Ok(())
    }
}
