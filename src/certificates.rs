use std::fs::File;
use std::io;

use openssl::x509::{X509, X509Name, X509NameRef};
use openssl::asn1::Asn1Time;
use openssl::pkey::{PKey, Private};
use openssl::rsa::Rsa;
use openssl::hash::MessageDigest;
use openssl::bn::{BigNum, MsbOption};
use openssl::x509::extension::{AuthorityKeyIdentifier, SubjectAlternativeName};
use openssl::pkcs12::Pkcs12;

pub struct CA {
    pub(self) cert: X509,
    pub(self) key: PKey<Private>,
}

impl CA {
    pub fn load_from_pem_files(cert_file: &str, key_file: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let mut cert_file = File::open(cert_file)?;
        let mut cert: Vec<u8> = vec![];
        io::copy(&mut cert_file, &mut cert)?;
        let cert = X509::from_pem(&cert)?;

        let mut key_file = File::open(key_file)?;
        let mut key: Vec<u8> = vec![];
        io::copy(&mut key_file, &mut key)?;
        let key = PKey::from_rsa(Rsa::private_key_from_pem(&key)?)?;

        Ok(CA { cert, key })
    }
}

pub(crate) fn load_key_from_file(key_file: &str) -> Result<PKey<Private>, Box<dyn std::error::Error>> {
    let mut key_file = File::open(key_file)?;
    let mut key: Vec<u8> = vec![];
    io::copy(&mut key_file, &mut key)?;
    //TODO: this unwrap doesn't feel right, need to get the types to match up properly
    Ok(PKey::from_rsa(Rsa::private_key_from_pem(&key)?).unwrap())
}

pub(crate) fn native_identity(certificate: &X509, key: &PKey<Private>) -> native_tls::Identity {
    let pkcs = Pkcs12::builder().build(&"", &"", key, certificate).unwrap().to_der().unwrap();
    native_tls::Identity::from_pkcs12(&pkcs, &"").unwrap()
}


pub(crate) fn create_signed_certificate_for_domain(domain: &str, ca: &CA) -> Result<X509, Box<dyn std::error::Error>> {
    let mut cert_builder = X509::builder()?;

    let mut host_name = X509Name::builder()?;
    host_name
        .append_entry_by_text("CN", domain)?;
    let host_name = host_name.build();

    cert_builder.set_subject_name(&host_name)?;
    // TODO: why version 2 and not 3 since we use the v3 context later?
    cert_builder.set_version(2)?;
    cert_builder.set_not_before(&Asn1Time::days_from_now(0).unwrap())?;
    cert_builder.set_not_after(&Asn1Time::days_from_now(365).unwrap())?;

    let serial_number = {
        let mut serial_number = BigNum::new()?;
        serial_number.rand(159, MsbOption::MAYBE_ZERO, false)?;
        serial_number.to_asn1_integer()?
    };
    cert_builder.set_serial_number(&serial_number)?;


    let subject_alternative_name = SubjectAlternativeName::new()
        .dns(domain)
        .build(&cert_builder.x509v3_context(Some(&ca.cert), None))?;
    cert_builder.append_extension(subject_alternative_name)?;

    let authority_key_identifier = AuthorityKeyIdentifier::new()
        .keyid(false)
        .issuer(false)
        .build(&cert_builder.x509v3_context(Some(&ca.cert), None))?;
    cert_builder.append_extension(authority_key_identifier)?;

    cert_builder.set_issuer_name(&ca.cert.issuer_name())?;
    cert_builder.set_pubkey(&ca.key)?;
    cert_builder.sign(&ca.key, MessageDigest::sha256())?;


    Ok(cert_builder.build())
}

fn copy_name(in_name: &X509NameRef) -> X509Name {
    let mut copy = X509Name::builder().unwrap();
    for entry in in_name.entries() {
        copy.append_entry_by_nid(
            entry.object().nid(),
            entry.data().as_utf8().expect("Expected string as entry in name").as_ref(),
        ).expect("Failed to add entry by nid");
    }

    copy.build()
}

fn copy_alt_names(in_cert: &X509) -> Option<SubjectAlternativeName> {
    match in_cert.subject_alt_names() {
        Some(in_alt_names) => {
            let mut subject_alternative_name = SubjectAlternativeName::new();
            for gn in in_alt_names {
                if let Some(email) = gn.email() {
                    subject_alternative_name.email(email);
                } else if let Some(dns) = gn.dnsname() {
                    subject_alternative_name.dns(dns);
                } else if let Some(uri) = gn.uri() {
                    subject_alternative_name.uri(uri);
                } else if let Some(ipaddress) = gn.ipaddress() {
                    //TODO: The openssl library exposes .ipaddress -> &[u8] and the builder wants &str
                    //TODO: I have no idea whether this is u8 ascii representation of the ip or what so
                    //TODO: lets just go with it for now.
                    subject_alternative_name.ip(&String::from_utf8(ipaddress.to_vec())
                        .expect("ip address on certificate is not formatted as ascii"));
                }
            }
            Some(subject_alternative_name)
        }
        None => None
    }
}

pub(crate) fn spoof_certificate(certificate: &X509, ca: &CA) -> Result<X509, Box<dyn std::error::Error>> {
    let mut cert_builder = X509::builder()?;

    let name: &X509NameRef = certificate.subject_name();
    let host_name = copy_name(name);
    cert_builder.set_subject_name(&host_name)?;
    cert_builder.set_not_before(certificate.not_before())?;
    cert_builder.set_not_after(certificate.not_after())?;

    cert_builder.set_serial_number(certificate.serial_number())?;


    //TODO: is this supposed to be set based on the certificate given?
    cert_builder.set_version(3)?;

    if let Some(subject_alternative_name) = copy_alt_names(certificate) {
        let subject_alternative_name = subject_alternative_name.build(&cert_builder.x509v3_context(Some(&ca.cert), None))?;
        cert_builder.append_extension(subject_alternative_name)?;
    }

    let authority_key_identifier = AuthorityKeyIdentifier::new()
        .keyid(false)
        .issuer(false)
        .build(&cert_builder.x509v3_context(Some(&ca.cert), None))?;
    cert_builder.append_extension(authority_key_identifier)?;

    cert_builder.set_issuer_name(&ca.cert.issuer_name())?;
    cert_builder.set_pubkey(&ca.key)?;
    cert_builder.sign(&ca.key, MessageDigest::sha256())?;


    Ok(cert_builder.build())
}