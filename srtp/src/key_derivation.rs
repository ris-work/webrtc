use aes::cipher::generic_array::GenericArray;
use aes::cipher::NewBlockCipher;
use aes::{Aes128, BlockEncrypt};

use byteorder::{BigEndian, WriteBytesExt};
use std::io::BufWriter;

use crate::error::{Error, Result};

pub const LABEL_SRTP_ENCRYPTION: u8 = 0x00;
pub const LABEL_SRTP_AUTHENTICATION_TAG: u8 = 0x01;
pub const LABEL_SRTP_SALT: u8 = 0x02;
pub const LABEL_SRTCP_ENCRYPTION: u8 = 0x03;
pub const LABEL_SRTCP_AUTHENTICATION_TAG: u8 = 0x04;
pub const LABEL_SRTCP_SALT: u8 = 0x05;

pub(crate) const SRTCP_INDEX_SIZE: usize = 4;

pub(crate) fn aes_cm_key_derivation(
    label: u8,
    master_key: &[u8],
    master_salt: &[u8],
    index_over_kdr: usize,
    out_len: usize,
) -> Result<Vec<u8>> {
    if index_over_kdr != 0 {
        // 24-bit "index DIV kdr" must be xored to prf input.
        return Err(Error::UnsupportedIndexOverKdr);
    }

    // https://tools.ietf.org/html/rfc3711#appendix-B.3
    // The input block for AES-CM is generated by exclusive-oring the master salt with the
    // concatenation of the encryption key label 0x00 with (index DIV kdr),
    // - index is 'rollover count' and DIV is 'divided by'

    let n_master_key = master_key.len();
    let n_master_salt = master_salt.len();

    let mut prf_in = vec![0u8; n_master_key];
    prf_in[..n_master_salt].copy_from_slice(master_salt);

    prf_in[7] ^= label;

    //The resulting value is then AES encrypted using the master key to get the cipher key.
    let key = GenericArray::from_slice(master_key);
    let block = Aes128::new(key);

    let mut out = vec![0u8; ((out_len + n_master_key) / n_master_key) * n_master_key];
    for (i, n) in (0..out_len).step_by(n_master_key).enumerate() {
        //BigEndian.PutUint16(prfIn[nMasterKey-2:], i)
        prf_in[n_master_key - 2] = ((i >> 8) & 0xFF) as u8;
        prf_in[n_master_key - 1] = (i & 0xFF) as u8;

        out[n..n + n_master_key].copy_from_slice(&prf_in);
        let out_key = GenericArray::from_mut_slice(&mut out[n..n + n_master_key]);
        block.encrypt_block(out_key);
    }

    Ok(out[..out_len].to_vec())
}

/// Generate IV https://tools.ietf.org/html/rfc3711#section-4.1.1
/// where the 128-bit integer value IV SHALL be defined by the SSRC, the
/// SRTP packet index i, and the SRTP session salting key k_s, as below.
/// ROC = a 32-bit unsigned rollover counter (roc), which records how many
/// times the 16-bit RTP sequence number has been reset to zero after
/// passing through 65,535
/// ```nobuild
/// i = 2^16 * roc + SEQ
/// IV = (salt*2 ^ 16) | (ssrc*2 ^ 64) | (i*2 ^ 16)
/// ```
pub(crate) fn generate_counter(
    sequence_number: u16,
    rollover_counter: u32,
    ssrc: u32,
    session_salt: &[u8],
) -> Result<Vec<u8>> {
    assert!(session_salt.len() <= 16);

    let mut counter: Vec<u8> = vec![0; 16];
    {
        let mut writer = BufWriter::<&mut [u8]>::new(counter[4..].as_mut());
        writer.write_u32::<BigEndian>(ssrc)?;
        writer.write_u32::<BigEndian>(rollover_counter)?;
        writer.write_u32::<BigEndian>((sequence_number as u32) << 16)?;
    }

    for i in 0..session_salt.len() {
        counter[i] ^= session_salt[i];
    }

    Ok(counter)
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::protection_profile::*;

    #[test]
    fn test_valid_session_keys() -> Result<()> {
        // Key Derivation Test Vectors from https://tools.ietf.org/html/rfc3711#appendix-B.3
        let master_key = vec![
            0xE1, 0xF9, 0x7A, 0x0D, 0x3E, 0x01, 0x8B, 0xE0, 0xD6, 0x4F, 0xA3, 0x2C, 0x06, 0xDE,
            0x41, 0x39,
        ];
        let master_salt = vec![
            0x0E, 0xC6, 0x75, 0xAD, 0x49, 0x8A, 0xFE, 0xEB, 0xB6, 0x96, 0x0B, 0x3A, 0xAB, 0xE6,
        ];

        let expected_session_key = vec![
            0xC6, 0x1E, 0x7A, 0x93, 0x74, 0x4F, 0x39, 0xEE, 0x10, 0x73, 0x4A, 0xFE, 0x3F, 0xF7,
            0xA0, 0x87,
        ];
        let expected_session_salt = vec![
            0x30, 0xCB, 0xBC, 0x08, 0x86, 0x3D, 0x8C, 0x85, 0xD4, 0x9D, 0xB3, 0x4A, 0x9A, 0xE1,
        ];
        let expected_session_auth_tag = vec![
            0xCE, 0xBE, 0x32, 0x1F, 0x6F, 0xF7, 0x71, 0x6B, 0x6F, 0xD4, 0xAB, 0x49, 0xAF, 0x25,
            0x6A, 0x15, 0x6D, 0x38, 0xBA, 0xA4,
        ];

        let session_key = aes_cm_key_derivation(
            LABEL_SRTP_ENCRYPTION,
            &master_key,
            &master_salt,
            0,
            master_key.len(),
        )?;
        assert_eq!(
            session_key, expected_session_key,
            "Session Key:\n{session_key:?} \ndoes not match expected:\n{expected_session_key:?}\nMaster Key:\n{master_key:?}\nMaster Salt:\n{master_salt:?}\n",
        );

        let session_salt = aes_cm_key_derivation(
            LABEL_SRTP_SALT,
            &master_key,
            &master_salt,
            0,
            master_salt.len(),
        )?;
        assert_eq!(
            session_salt, expected_session_salt,
            "Session Salt {session_salt:?} does not match expected {expected_session_salt:?}"
        );

        let auth_key_len = ProtectionProfile::Aes128CmHmacSha1_80.auth_key_len();

        let session_auth_tag = aes_cm_key_derivation(
            LABEL_SRTP_AUTHENTICATION_TAG,
            &master_key,
            &master_salt,
            0,
            auth_key_len,
        )?;
        assert_eq!(
            session_auth_tag, expected_session_auth_tag,
            "Session Auth Tag {session_auth_tag:?} does not match expected {expected_session_auth_tag:?}",
        );

        Ok(())
    }

    // This test asserts that calling aesCmKeyDerivation with a non-zero indexOverKdr fails
    // Currently this isn't supported, but the API makes sure we can add this in the future
    #[test]
    fn test_index_over_kdr() -> Result<()> {
        let result = aes_cm_key_derivation(LABEL_SRTP_AUTHENTICATION_TAG, &[], &[], 1, 0);
        assert!(result.is_err());

        Ok(())
    }
}
