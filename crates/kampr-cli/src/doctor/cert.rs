//! Just enough X.509 to answer "has it expired".
//!
//! A whole ASN.1 stack for two timestamps would be a dependency that grants code execution on
//! this host for the sake of one line of doctor output. The walk below is fixed by the standard:
//! Certificate ::= SEQUENCE { tbsCertificate SEQUENCE { [0] version?, serialNumber INTEGER,
//! signature SEQUENCE, issuer SEQUENCE, validity SEQUENCE { notBefore, notAfter }, ... } }.

use anyhow::{Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use std::path::Path;
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time};

const SEQUENCE: u8 = 0x30;
const INTEGER: u8 = 0x02;
const CONTEXT_0: u8 = 0xA0;
const UTC_TIME: u8 = 0x17;
const GENERALIZED_TIME: u8 = 0x18;

/// Whole days until the first certificate in the file expires. Negative once it has.
pub fn expiry(path: &Path) -> Result<i64> {
    let pem = std::fs::read_to_string(path)?;
    let seconds = not_after(&der(&pem)?)?;
    Ok((seconds - OffsetDateTime::now_utc().unix_timestamp()) / 86_400)
}

fn der(pem: &str) -> Result<Vec<u8>> {
    let body: String = pem
        .lines()
        .skip_while(|l| !l.starts_with("-----BEGIN CERTIFICATE"))
        .skip(1)
        .take_while(|l| !l.starts_with("-----END CERTIFICATE"))
        .collect();
    if body.is_empty() {
        bail!("no PEM certificate block");
    }
    Ok(STANDARD.decode(body.trim())?)
}

fn not_after(der: &[u8]) -> Result<i64> {
    let tbs = contents(der, SEQUENCE).and_then(|c| contents(c, SEQUENCE))?;
    let mut rest = tbs;
    if rest.first() == Some(&CONTEXT_0) {
        rest = after(rest)?;
    }
    for tag in [INTEGER, SEQUENCE, SEQUENCE] {
        match rest.first() {
            Some(&found) if found == tag => {}
            Some(&found) => bail!("unexpected tag {found:#04x} where {tag:#04x} belongs"),
            None => bail!("the certificate ends where {tag:#04x} belongs"),
        }
        rest = after(rest)?;
    }
    let validity = contents(rest, SEQUENCE)?;
    timestamp(after(validity)?)
}

/// The value bytes of the element at the head of `input`, after checking its tag.
fn contents(input: &[u8], tag: u8) -> Result<&[u8]> {
    let (found, body, _) = element(input)?;
    if found != tag {
        bail!("expected tag {tag:#04x}, found {found:#04x}");
    }
    Ok(body)
}

/// Everything after the element at the head of `input`.
fn after(input: &[u8]) -> Result<&[u8]> {
    Ok(element(input)?.2)
}

fn element(input: &[u8]) -> Result<(u8, &[u8], &[u8])> {
    let (&tag, rest) = input.split_first().ok_or_else(|| anyhow::anyhow!("truncated"))?;
    let (&first, rest) = rest.split_first().ok_or_else(|| anyhow::anyhow!("truncated"))?;
    let (len, rest) = if first < 0x80 {
        (first as usize, rest)
    } else {
        let count = (first & 0x7f) as usize;
        if count == 0 || count > 4 || rest.len() < count {
            bail!("unsupported length encoding");
        }
        let (bytes, rest) = rest.split_at(count);
        (bytes.iter().fold(0usize, |n, b| (n << 8) | *b as usize), rest)
    };
    if rest.len() < len {
        bail!("truncated value");
    }
    let (body, tail) = rest.split_at(len);
    Ok((tag, body, tail))
}

fn timestamp(input: &[u8]) -> Result<i64> {
    let (tag, body, _) = element(input)?;
    let text = std::str::from_utf8(body)?.trim_end_matches('Z');
    let (year, rest) = match tag {
        // A two-digit year is the century that ends in 2049, by RFC 5280.
        UTC_TIME => {
            let year: i32 = text.get(..2).unwrap_or_default().parse()?;
            (if year >= 50 { 1900 + year } else { 2000 + year }, &text[2..])
        }
        GENERALIZED_TIME => (text.get(..4).unwrap_or_default().parse()?, &text[4..]),
        other => bail!("{other:#04x} is not a time"),
    };
    if rest.len() < 10 {
        bail!("truncated time {text}");
    }
    let field = |at: usize| -> Result<u8> {
        let digits = rest
            .get(at..at + 2)
            .ok_or_else(|| anyhow::anyhow!("truncated time {text}"))?;
        Ok(digits.parse()?)
    };
    let date = Date::from_calendar_date(year, Month::try_from(field(0)?)?, field(2)?)?;
    let time = Time::from_hms(field(4)?, field(6)?, field(8)?)?;
    Ok(PrimitiveDateTime::new(date, time).assume_utc().unix_timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::Duration;

    fn write_cert(dir: &Path, name: &str, valid_for: Duration) -> std::path::PathBuf {
        let key = rcgen::KeyPair::generate().expect("key");
        let mut params = rcgen::CertificateParams::new(vec!["kampr.example.com".into()]).expect("params");
        params.not_before = OffsetDateTime::now_utc() - Duration::days(1);
        params.not_after = OffsetDateTime::now_utc() + valid_for;
        let cert = params.self_signed(&key).expect("cert");
        let path = dir.join(name);
        std::fs::write(&path, cert.pem()).expect("write");
        path
    }

    #[test]
    fn a_live_certificate_reports_the_days_it_has_left() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_cert(dir.path(), "cert.pem", Duration::days(90) + Duration::hours(12));
        assert_eq!(expiry(&path).unwrap(), 90);
    }

    #[test]
    fn an_expired_certificate_reports_a_negative_span() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_cert(dir.path(), "old.pem", Duration::days(-30));
        assert!(expiry(&path).unwrap() <= -30, "{}", expiry(&path).unwrap());
    }

    #[test]
    fn anything_that_is_not_a_certificate_is_an_error_rather_than_a_guess() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key.pem");
        std::fs::write(
            &path,
            "-----BEGIN PRIVATE KEY-----\nMIIB\n-----END PRIVATE KEY-----\n",
        )
        .unwrap();
        assert!(expiry(&path).is_err());
        assert!(expiry(&dir.path().join("absent.pem")).is_err());
    }

    /// `kampr doctor` is the command an operator runs when something is already wrong, and
    /// `exposure` is written to turn this error into a `tls` finding. A panic here takes the
    /// whole report with it.
    #[test]
    fn a_truncated_certificate_fails_the_check_instead_of_aborting_doctor() {
        let dir = tempfile::tempdir().unwrap();
        // SEQUENCE { SEQUENCE {} } — a certificate that runs out exactly where the serial number
        // belongs, which is what a half-written or truncated PEM looks like.
        let path = dir.path().join("stub.pem");
        std::fs::write(
            &path,
            "-----BEGIN CERTIFICATE-----\nMAIwAA==\n-----END CERTIFICATE-----\n",
        )
        .unwrap();
        assert!(expiry(&path).is_err());

        let garbage = dir.path().join("garbage.pem");
        std::fs::write(
            &garbage,
            "-----BEGIN CERTIFICATE-----\nAAAAAAAAAAAAAAAAAAAAAAAA\n-----END CERTIFICATE-----\n",
        )
        .unwrap();
        assert!(expiry(&garbage).is_err());
    }
}
