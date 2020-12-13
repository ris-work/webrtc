#[cfg(test)]
mod crypto_test;

pub mod crypto_cbc;
pub mod crypto_ccm;
pub mod crypto_gcm;

use crate::curve::named_curve::*;
use crate::errors::*;
use crate::record_layer::record_layer_header::*;

use der_parser::{oid, oid::Oid};

use util::Error;

use ring::rand::SystemRandom;
use ring::signature::{EcdsaKeyPair, Ed25519KeyPair, RsaKeyPair};

use sha2::{Digest, Sha256};

//use log::*;

#[derive(Clone)]
pub struct Certificate {
    pub certificate: Vec<u8>,
    pub private_key: CryptoPrivateKey,
}

impl Certificate {
    pub fn generate_self_signed(subject_alt_names: impl Into<Vec<String>>) -> Result<Self, Error> {
        let cert = rcgen::generate_simple_self_signed(subject_alt_names)?;
        let certificate = cert.serialize_der()?;
        let key_pair = cert.get_key_pair();
        let serialized_der = key_pair.serialize_der();
        let private_key = if key_pair.is_compatible(&rcgen::PKCS_ED25519) {
            CryptoPrivateKey {
                kind: CryptoPrivateKeyKind::ED25519(Ed25519KeyPair::from_pkcs8(&serialized_der)?),
                serialized_der,
            }
        } else if key_pair.is_compatible(&rcgen::PKCS_ECDSA_P256_SHA256) {
            CryptoPrivateKey {
                kind: CryptoPrivateKeyKind::ECDSA256(EcdsaKeyPair::from_pkcs8(
                    &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
                    &serialized_der,
                )?),
                serialized_der,
            }
        } else if key_pair.is_compatible(&rcgen::PKCS_RSA_SHA256) {
            CryptoPrivateKey {
                kind: CryptoPrivateKeyKind::RSA256(RsaKeyPair::from_pkcs8(&serialized_der)?),
                serialized_der,
            }
        } else {
            return Err(Error::new("Unsupported key_pair".to_owned()));
        };

        Ok(Certificate {
            certificate,
            private_key,
        })
    }

    pub fn generate_self_signed_with_alg(
        subject_alt_names: impl Into<Vec<String>>,
        alg: &'static rcgen::SignatureAlgorithm,
    ) -> Result<Self, Error> {
        let mut params = rcgen::CertificateParams::new(subject_alt_names);
        params.alg = alg;
        let cert = rcgen::Certificate::from_params(params)?;
        let certificate = cert.serialize_der()?;
        let key_pair = cert.get_key_pair();
        let serialized_der = key_pair.serialize_der();
        let private_key = if key_pair.is_compatible(&rcgen::PKCS_ED25519) {
            CryptoPrivateKey {
                kind: CryptoPrivateKeyKind::ED25519(Ed25519KeyPair::from_pkcs8(&serialized_der)?),
                serialized_der,
            }
        } else if key_pair.is_compatible(&rcgen::PKCS_ECDSA_P256_SHA256) {
            CryptoPrivateKey {
                kind: CryptoPrivateKeyKind::ECDSA256(EcdsaKeyPair::from_pkcs8(
                    &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
                    &serialized_der,
                )?),
                serialized_der,
            }
        } else if key_pair.is_compatible(&rcgen::PKCS_RSA_SHA256) {
            CryptoPrivateKey {
                kind: CryptoPrivateKeyKind::RSA256(RsaKeyPair::from_pkcs8(&serialized_der)?),
                serialized_der,
            }
        } else {
            return Err(Error::new("Unsupported key_pair".to_owned()));
        };

        Ok(Certificate {
            certificate,
            private_key,
        })
    }
}

pub(crate) fn value_key_message(
    client_random: &[u8],
    server_random: &[u8],
    public_key: &[u8],
    named_curve: NamedCurve,
) -> Vec<u8> {
    let mut server_ecdh_params = vec![0u8; 4];
    server_ecdh_params[0] = 3; // named curve
    server_ecdh_params[1..3].copy_from_slice(&(named_curve as u16).to_be_bytes());
    server_ecdh_params[3] = public_key.len() as u8;

    let mut plaintext = vec![];
    plaintext.extend_from_slice(client_random);
    plaintext.extend_from_slice(server_random);
    plaintext.extend_from_slice(&server_ecdh_params);
    plaintext.extend_from_slice(public_key);

    plaintext
}

pub(crate) enum CryptoPrivateKeyKind {
    ED25519(Ed25519KeyPair),
    ECDSA256(EcdsaKeyPair),
    RSA256(RsaKeyPair),
}

pub struct CryptoPrivateKey {
    pub(crate) kind: CryptoPrivateKeyKind,
    pub(crate) serialized_der: Vec<u8>,
}

impl Clone for CryptoPrivateKey {
    fn clone(&self) -> Self {
        match self.kind {
            CryptoPrivateKeyKind::ED25519(_) => CryptoPrivateKey {
                kind: CryptoPrivateKeyKind::ED25519(
                    Ed25519KeyPair::from_pkcs8(&self.serialized_der).unwrap(),
                ),
                serialized_der: self.serialized_der.clone(),
            },
            CryptoPrivateKeyKind::ECDSA256(_) => CryptoPrivateKey {
                kind: CryptoPrivateKeyKind::ECDSA256(
                    EcdsaKeyPair::from_pkcs8(
                        &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
                        &self.serialized_der,
                    )
                    .unwrap(),
                ),
                serialized_der: self.serialized_der.clone(),
            },
            CryptoPrivateKeyKind::RSA256(_) => CryptoPrivateKey {
                kind: CryptoPrivateKeyKind::RSA256(
                    RsaKeyPair::from_pkcs8(&self.serialized_der).unwrap(),
                ),
                serialized_der: self.serialized_der.clone(),
            },
        }
    }
}

// If the client provided a "signature_algorithms" extension, then all
// certificates provided by the server MUST be signed by a
// hash/signature algorithm pair that appears in that extension
//
// https://tools.ietf.org/html/rfc5246#section-7.4.2
pub(crate) fn generate_key_signature(
    client_random: &[u8],
    server_random: &[u8],
    public_key: &[u8],
    named_curve: NamedCurve,
    private_key: &CryptoPrivateKey, /*, hash_algorithm: HashAlgorithm*/
) -> Result<Vec<u8>, Error> {
    let msg = value_key_message(client_random, server_random, public_key, named_curve);
    let signature = match &private_key.kind {
        CryptoPrivateKeyKind::ED25519(kp) => kp.sign(&msg).as_ref().to_vec(),
        CryptoPrivateKeyKind::ECDSA256(kp) => {
            let system_random = SystemRandom::new();
            kp.sign(&system_random, &msg)?.as_ref().to_vec()
        }
        CryptoPrivateKeyKind::RSA256(kp) => {
            let system_random = SystemRandom::new();
            let mut signature = vec![0; kp.public_modulus_len()];
            kp.sign(
                &ring::signature::RSA_PKCS1_SHA256,
                &system_random,
                &msg,
                &mut signature,
            )?;

            signature
        }
    };

    Ok(signature)
}

// add OID_ED25519 which is not defined in x509_parser
pub const OID_ED25519: Oid<'static> = oid!(1.3.101 .112);
pub const OID_ECDSA: Oid<'static> = oid!(1.2.840 .10045 .2 .1);

pub(crate) fn verify_key_signature(
    message: &[u8],
    /*_hash_algorithm: HashAlgorithm,*/
    remote_key_signature: &[u8],
    raw_certificates: &[u8],
) -> Result<(), Error> {
    if raw_certificates.is_empty() {
        return Err(ERR_LENGTH_MISMATCH.clone());
    }

    let (_, certificate) = x509_parser::parse_x509_der(raw_certificates)?;

    let pki_alg = &certificate.tbs_certificate.subject_pki.algorithm.algorithm;
    let sign_alg = &certificate.tbs_certificate.signature.algorithm;

    let verify_alg: &dyn ring::signature::VerificationAlgorithm = if *pki_alg == OID_ED25519 {
        &ring::signature::ED25519
    } else if *pki_alg == OID_ECDSA {
        if *sign_alg == x509_parser::objects::OID_ECDSA_SHA256 {
            &ring::signature::ECDSA_P256_SHA256_ASN1
        } else if *sign_alg == x509_parser::objects::OID_ECDSA_SHA384 {
            &ring::signature::ECDSA_P384_SHA384_ASN1
        } else {
            return Err(ERR_KEY_SIGNATURE_VERIFY_UNIMPLEMENTED.clone());
        }
    } else if *pki_alg == x509_parser::objects::OID_RSA_ENCRYPTION {
        if *sign_alg == x509_parser::objects::OID_RSA_SHA1 {
            &ring::signature::RSA_PKCS1_1024_8192_SHA1_FOR_LEGACY_USE_ONLY
        } else if *sign_alg == x509_parser::objects::OID_RSA_SHA256 {
            &ring::signature::RSA_PKCS1_2048_8192_SHA256
        } else if *sign_alg == x509_parser::objects::OID_RSA_SHA384 {
            &ring::signature::RSA_PKCS1_2048_8192_SHA384
        } else if *sign_alg == x509_parser::objects::OID_RSA_SHA512 {
            &ring::signature::RSA_PKCS1_2048_8192_SHA512
        } else {
            return Err(ERR_KEY_SIGNATURE_VERIFY_UNIMPLEMENTED.clone());
        }
    } else {
        return Err(ERR_KEY_SIGNATURE_VERIFY_UNIMPLEMENTED.clone());
    };

    let public_key = ring::signature::UnparsedPublicKey::new(
        verify_alg,
        certificate
            .tbs_certificate
            .subject_pki
            .subject_public_key
            .data,
    );

    public_key.verify(&message, remote_key_signature)?;

    Ok(())
}

// If the server has sent a CertificateRequest message, the client MUST send the Certificate
// message.  The ClientKeyExchange message is now sent, and the content
// of that message will depend on the public key algorithm selected
// between the ClientHello and the ServerHello.  If the client has sent
// a certificate with signing ability, a digitally-signed
// CertificateVerify message is sent to explicitly verify possession of
// the private key in the certificate.
// https://tools.ietf.org/html/rfc5246#section-7.3
pub(crate) fn generate_certificate_verify(
    handshake_bodies: &[u8],
    private_key: &CryptoPrivateKey, /*, hashAlgorithm hashAlgorithm*/
) -> Result<Vec<u8>, Error> {
    let mut h = Sha256::new();
    h.update(handshake_bodies);
    let hashed = h.finalize();

    let signature = match &private_key.kind {
        CryptoPrivateKeyKind::ED25519(kp) => kp.sign(hashed.as_slice()).as_ref().to_vec(),
        CryptoPrivateKeyKind::ECDSA256(kp) => {
            let system_random = SystemRandom::new();
            kp.sign(&system_random, hashed.as_slice())?
                .as_ref()
                .to_vec()
        }
        CryptoPrivateKeyKind::RSA256(kp) => {
            let system_random = SystemRandom::new();
            let mut signature = vec![0; kp.public_modulus_len()];
            kp.sign(
                &ring::signature::RSA_PKCS1_SHA256,
                &system_random,
                hashed.as_slice(),
                &mut signature,
            )?;

            signature
        }
    };

    Ok(signature)
}

pub(crate) fn verify_certificate_verify(
    handshake_bodies: &[u8],
    /*hashAlgorithm hashAlgorithm,*/
    remote_key_signature: &[u8],
    raw_certificates: &[u8],
) -> Result<(), Error> {
    let mut h = Sha256::new();
    h.update(handshake_bodies);
    let hashed = h.finalize();

    verify_key_signature(&hashed, remote_key_signature, raw_certificates)
}

pub(crate) fn load_certs(
    raw_certificates: &[u8],
) -> Result<x509_parser::X509Certificate<'_>, Error> {
    if raw_certificates.is_empty() {
        return Err(ERR_LENGTH_MISMATCH.clone());
    }

    let (_, certificate) = x509_parser::parse_x509_der(raw_certificates)?;

    Ok(certificate)
}

pub(crate) fn verify_cert(
    raw_certificates: &[u8],
) -> Result<Vec<x509_parser::X509Certificate<'_>>, Error> {
    let certificate = load_certs(raw_certificates)?;

    certificate.verify_signature(None)?;

    Ok(vec![certificate])
}

pub(crate) fn generate_aead_additional_data(h: &RecordLayerHeader, payload_len: usize) -> Vec<u8> {
    let mut additional_data = vec![0u8; 13];
    // SequenceNumber MUST be set first
    // we only want uint48, clobbering an extra 2 (using uint64, rust doesn't have uint48)
    additional_data[..8].copy_from_slice(&h.sequence_number.to_be_bytes());
    additional_data[..2].copy_from_slice(&h.epoch.to_be_bytes());
    additional_data[8] = h.content_type as u8;
    additional_data[9] = h.protocol_version.major;
    additional_data[10] = h.protocol_version.minor;
    additional_data[11..].copy_from_slice(&(payload_len as u16).to_be_bytes());

    additional_data
}
