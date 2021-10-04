use crate::error::Error;
use crate::nack::UINT16SIZE_HALF;
use anyhow::Result;

#[derive(Default, Debug)]
struct ReceiveLog {
    packets: Vec<u64>,
    size: u16,
    end: u16,
    started: bool,
    last_consecutive: u16,
}

impl ReceiveLog {
    fn new(size: u16) -> Result<Self> {
        let mut correct_size = false;
        for i in 6..16 {
            if size == (1 << i) {
                correct_size = true;
                break;
            }
        }

        if !correct_size {
            return Err(Error::ErrInvalidSize.into());
        }

        Ok(ReceiveLog {
            packets: vec![0u64; (size as usize) / 64],
            size,
            ..Default::default()
        })
    }

    fn add(&mut self, seq: u16) {
        if !self.started {
            self.set_received(seq);
            self.end = seq;
            self.started = true;
            self.last_consecutive = seq;
            return;
        }

        let (last_consecutive_plus1, _) = self.last_consecutive.overflowing_add(1);
        let (diff, _) = seq.overflowing_sub(self.end);
        if diff == 0 {
            return;
        } else if diff < UINT16SIZE_HALF {
            // this means a positive diff, in other words seq > end (with counting for rollovers)
            let (mut i, _) = self.end.overflowing_add(1);
            while i != seq {
                // clear packets between end and seq (these may contain packets from a "size" ago)
                self.del_received(i);
                let (j, _) = i.overflowing_add(1);
                i = j;
            }
            self.end = seq;

            let (seq_sub_last_consecutive, _) = seq.overflowing_sub(self.last_consecutive);
            if last_consecutive_plus1 == seq {
                self.last_consecutive = seq;
            } else if seq_sub_last_consecutive > self.size {
                let (diff, _) = seq.overflowing_sub(self.size);
                self.last_consecutive = diff;
                self.fix_last_consecutive(); // there might be valid packets at the beginning of the buffer now
            }
        } else if last_consecutive_plus1 == seq {
            // negative diff, seq < end (with counting for rollovers)
            self.last_consecutive = seq;
            self.fix_last_consecutive(); // there might be other valid packets after seq
        }

        self.set_received(seq);
    }

    fn get(&self, seq: u16) -> bool {
        let (diff, _) = self.end.overflowing_sub(seq);
        if diff >= UINT16SIZE_HALF {
            return false;
        }

        if diff >= self.size {
            return false;
        }

        self.get_received(seq)
    }

    fn missing_seq_numbers(&self, skip_last_n: u16) -> Vec<u16> {
        let (until, _) = self.end.overflowing_sub(skip_last_n);
        let (diff, _) = until.overflowing_sub(self.last_consecutive);
        if diff >= UINT16SIZE_HALF {
            // until < s.last_consecutive (counting for rollover)
            return vec![];
        }

        let mut missing_packet_seq_nums = vec![];
        let (mut i, _) = self.last_consecutive.overflowing_add(1);
        let (util_plus1, _) = until.overflowing_add(1);
        while i != util_plus1 {
            if !self.get_received(i) {
                missing_packet_seq_nums.push(i);
            }
            let (j, _) = i.overflowing_add(1);
            i = j;
        }

        missing_packet_seq_nums
    }

    fn set_received(&mut self, seq: u16) {
        let pos = (seq % self.size) as usize;
        self.packets[pos / 64] |= 1u64 << (pos % 64);
    }

    fn del_received(&mut self, seq: u16) {
        let pos = (seq % self.size) as usize;
        self.packets[pos / 64] &= u64::MAX ^ (1u64 << (pos % 64));
    }

    fn get_received(&self, seq: u16) -> bool {
        let pos = (seq % self.size) as usize;
        (self.packets[pos / 64] & (1u64 << (pos % 64))) != 0
    }

    fn fix_last_consecutive(&mut self) {
        let mut i = self.last_consecutive + 1;
        while i != self.end + 1 && self.get_received(i) {
            // find all consecutive packets
            i += 1;
        }
        self.last_consecutive = i - 1;
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_received_buffer() -> Result<()> {
        let tests: Vec<u16> = vec![
            0, 1, 127, 128, 129, 511, 512, 513, 32767, 32768, 32769, 65407, 65408, 65409, 65534,
            65535,
        ];
        for start in tests {
            let mut rl = ReceiveLog::new(128)?;

            let all = |min: u16, max: u16| -> Vec<u16> {
                let mut result = vec![];
                let mut i = min;
                let (max_plus_1, _) = max.overflowing_add(1);
                while i != max_plus_1 {
                    result.push(i);
                    let (j, _) = i.overflowing_add(1);
                    i = j;
                }
                result
            };

            let join = |parts: &[&[u16]]| -> Vec<u16> {
                let mut result = vec![];
                for p in parts {
                    result.extend_from_slice(*p);
                }
                result
            };

            let add = |rl: &mut ReceiveLog, nums: &[u16]| {
                for n in nums {
                    let (seq, _) = start.overflowing_add(*n);
                    rl.add(seq);
                }
            };

            let assert_get = |rl: &ReceiveLog, nums: &[u16]| {
                for n in nums {
                    let (seq, _) = start.overflowing_add(*n);
                    assert!(rl.get(seq), "not found: {}", seq);
                }
            };

            let assert_not_get = |rl: &ReceiveLog, nums: &[u16]| {
                for n in nums {
                    let (seq, _) = start.overflowing_add(*n);
                    assert!(
                        !rl.get(seq),
                        "packet found: start {}, n {}, seq {}",
                        start,
                        *n,
                        seq
                    );
                }
            };

            let assert_missing = |rl: &ReceiveLog, skip_last_n: u16, nums: &[u16]| {
                let missing = rl.missing_seq_numbers(skip_last_n);
                let mut want = vec![];
                for n in nums {
                    let (seq, _) = start.overflowing_add(*n);
                    want.push(seq);
                }
                assert_eq!(want, missing, "missing want/got, ");
            };

            let assert_last_consecutive = |rl: &ReceiveLog, last_consecutive: u16| {
                let (want, _) = last_consecutive.overflowing_add(start);
                assert_eq!(rl.last_consecutive, want, "invalid last_consecutive want");
            };

            add(&mut rl, &[0]);
            assert_get(&rl, &[0]);
            assert_missing(&rl, 0, &[]);
            assert_last_consecutive(&rl, 0); // first element added

            add(&mut rl, &all(1, 127));
            assert_get(&rl, &all(1, 127));
            assert_missing(&rl, 0, &[]);
            assert_last_consecutive(&rl, 127);

            add(&mut rl, &[128]);
            assert_get(&rl, &[128]);
            assert_not_get(&rl, &[0]);
            assert_missing(&rl, 0, &[]);
            assert_last_consecutive(&rl, 128);

            add(&mut rl, &[130]);
            assert_get(&rl, &[130]);
            assert_not_get(&rl, &[1, 2, 129]);
            assert_missing(&rl, 0, &[129]);
            assert_last_consecutive(&rl, 128);

            add(&mut rl, &[333]);
            assert_get(&rl, &[333]);
            assert_not_get(&rl, &all(0, 332));
            assert_missing(&rl, 0, &all(206, 332)); // all 127 elements missing before 333
            assert_missing(&rl, 10, &all(206, 323)); // skip last 10 packets (324-333) from check
            assert_last_consecutive(&rl, 205); // lastConsecutive is still out of the buffer

            add(&mut rl, &[329]);
            assert_get(&rl, &[329]);
            assert_missing(&rl, 0, &join(&[&all(206, 328), &all(330, 332)]));
            assert_missing(&rl, 5, &join(&[&all(206, 328)])); // skip last 5 packets (329-333) from check
            assert_last_consecutive(&rl, 205);

            add(&mut rl, &all(207, 320));
            assert_get(&rl, &all(207, 320));
            assert_missing(&rl, 0, &join(&[&[206], &all(321, 328), &all(330, 332)]));
            assert_last_consecutive(&rl, 205);

            add(&mut rl, &[334]);
            assert_get(&rl, &[334]);
            assert_not_get(&rl, &[206]);
            assert_missing(&rl, 0, &join(&[&all(321, 328), &all(330, 332)]));
            assert_last_consecutive(&rl, 320); // head of buffer is full of consecutive packages

            add(&mut rl, &all(322, 328));
            assert_get(&rl, &all(322, 328));
            assert_missing(&rl, 0, &join(&[&[321], &all(330, 332)]));
            assert_last_consecutive(&rl, 320);

            add(&mut rl, &[321]);
            assert_get(&rl, &[321]);
            assert_missing(&rl, 0, &all(330, 332));
            assert_last_consecutive(&rl, 329); // after adding a single missing packet, lastConsecutive should jump forward
        }

        Ok(())
    }
}
