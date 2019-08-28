use super::*;

use std::io::{BufReader, BufWriter};

use util::Error;

#[test]
fn test_transport_layer_nack_unmarshal() -> Result<(), Error> {
    let tests = vec![
        (
            "valid",
            vec![
                // TransportLayerNack
                0x81, 0xcd, 0x0, 0x3, // sender=0x902f9e2e
                0x90, 0x2f, 0x9e, 0x2e, // media=0x902f9e2e
                0x90, 0x2f, 0x9e, 0x2e, // nack 0xAAAA, 0x5555
                0xaa, 0xaa, 0x55, 0x55,
            ],
            TransportLayerNack {
                sender_ssrc: 0x902f9e2e,
                media_ssrc: 0x902f9e2e,
                nacks: vec![NackPair {
                    packet_id: 0xaaaa,
                    lost_packets: 0x5555,
                }],
            },
            None,
        ),
        (
            "short report",
            vec![
                0x81, 0xcd, 0x0, 0x2, // ssrc=0x902f9e2e
                0x90, 0x2f, 0x9e, 0x2e,
                // report ends early
            ],
            TransportLayerNack::default(),
            Some(ErrFailedToFillWholeBuffer.clone()),
        ),
        (
            "wrong type",
            vec![
                // v=2, p=0, count=1, SR, len=7
                0x81, 0xc8, 0x0, 0x7, // ssrc=0x902f9e2e
                0x90, 0x2f, 0x9e, 0x2e, // ssrc=0xbc5e9a40
                0xbc, 0x5e, 0x9a, 0x40, // fracLost=0, totalLost=0
                0x0, 0x0, 0x0, 0x0, // lastSeq=0x46e1
                0x0, 0x0, 0x46, 0xe1, // jitter=273
                0x0, 0x0, 0x1, 0x11, // lsr=0x9f36432
                0x9, 0xf3, 0x64, 0x32, // delay=150137
                0x0, 0x2, 0x4a, 0x79,
            ],
            TransportLayerNack::default(),
            Some(ErrWrongType.clone()),
        ),
        (
            "nil",
            vec![],
            TransportLayerNack::default(),
            Some(ErrFailedToFillWholeBuffer.clone()),
        ),
    ];

    for (name, data, want, want_error) in tests {
        let mut reader = BufReader::new(data.as_slice());
        let result = TransportLayerNack::unmarshal(&mut reader);
        if let Some(err) = want_error {
            if let Err(got) = result {
                assert_eq!(
                    got, err,
                    "Unmarshal {} header: err = {}, want {}",
                    name, got, err
                );
            } else {
                assert!(false, "want error in test {}", name);
            }
        } else {
            if let Ok(got) = result {
                assert_eq!(
                    got, want,
                    "Unmarshal {} header: got {:?}, want {:?}",
                    name, got, want,
                )
            } else {
                assert!(false, "must no error in test {}", name);
            }
        }
    }

    Ok(())
}

#[test]
fn test_transport_layer_nack_roundtrip() -> Result<(), Error> {
    let tests = vec![(
        "valid",
        TransportLayerNack {
            sender_ssrc: 0x902f9e2e,
            media_ssrc: 0x902f9e2e,
            nacks: vec![
                NackPair {
                    packet_id: 1,
                    lost_packets: 0xAA,
                },
                NackPair {
                    packet_id: 1034,
                    lost_packets: 0x05,
                },
            ],
        },
        None,
    )];

    for (name, report, marshal_error) in tests {
        let mut data: Vec<u8> = vec![];
        {
            let mut writer = BufWriter::<&mut Vec<u8>>::new(data.as_mut());
            let result = report.marshal(&mut writer);
            if let Some(err) = marshal_error {
                if let Err(got) = result {
                    assert_eq!(
                        got, err,
                        "marshal {} header: err = {}, want {}",
                        name, got, err
                    );
                } else {
                    assert!(false, "want error in test {}", name);
                }
                continue;
            } else {
                assert!(result.is_ok(), "must no error in test {}", name);
            }
        }

        let mut reader = BufReader::new(data.as_slice());
        let decoded = TransportLayerNack::unmarshal(&mut reader)?;
        assert_eq!(
            decoded, report,
            "{} header round trip: got {:?}, want {:?}",
            name, decoded, report
        )
    }

    Ok(())
}
