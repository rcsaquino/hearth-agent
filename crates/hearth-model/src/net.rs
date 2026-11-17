//! Outbound address policy.
//!
//! The URL check and the connection check share one predicate. A name is
//! resolved twice by reqwest, so the same filter is installed as the client's
//! resolver and applied when the connection is actually made.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use reqwest::Url;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};

/// Whether an address may be contacted for untrusted content.
pub fn address_allowed(address: IpAddr, allow_loopback: bool) -> bool {
    if allow_loopback && address.is_loopback() {
        return true;
    }
    !is_disallowed(address)
}

/// Check a URL before a request and every redirect.
pub fn validate_url(url: &Url, allow_loopback: bool) -> Result<(), String> {
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err("only http and https URLs are allowed".to_owned());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("URLs with user information are not allowed".to_owned());
    }
    let host = url.host_str().ok_or_else(|| "the URL has no host".to_owned())?;
    let host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "the URL has no usable port".to_owned())?;
    if let Ok(address) = host.parse::<IpAddr>() {
        if !address_allowed(address, allow_loopback) {
            return Err(format!("address {address} is not public"));
        }
        return Ok(());
    }
    let mut found = false;
    for address in (host, port)
        .to_socket_addrs()
        .map_err(|error| format!("could not resolve {host}: {error}"))?
    {
        found = true;
        if !address_allowed(address.ip(), allow_loopback) {
            return Err(format!("address {} is not public", address.ip()));
        }
    }
    if !found {
        return Err(format!("could not resolve {host}"));
    }
    Ok(())
}

/// A resolver that drops an address the product does not contact.
pub fn public_resolver(allow_loopback: bool) -> PublicResolver {
    PublicResolver { allow_loopback }
}

/// A resolver that keeps only the addresses the product may contact.
#[derive(Debug)]
pub struct PublicResolver {
    allow_loopback: bool,
}

impl Resolve for PublicResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        let allow_loopback = self.allow_loopback;
        Box::pin(async move {
            let addrs = (host.as_str(), 0)
                .to_socket_addrs()
                .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
                    Box::new(error)
                })?;
            let resolved: Vec<SocketAddr> = addrs
                .filter(|address| address_allowed(address.ip(), allow_loopback))
                .collect();
            if resolved.is_empty() {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("{host} does not resolve to a public address"),
                )) as Box<dyn std::error::Error + Send + Sync>);
            }
            Ok(Box::new(resolved.into_iter()) as Addrs)
        })
    }
}

fn is_disallowed(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_disallowed_v4(address),
        IpAddr::V6(address) => {
            if let Some(mapped) = address.to_ipv4_mapped() {
                return is_disallowed_v4(mapped);
            }
            let segments = address.segments();
            address.is_loopback()
                || address.is_unspecified()
                || address.is_multicast()
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        }
    }
}

fn is_disallowed_v4(address: Ipv4Addr) -> bool {
    let octets = address.octets();
    address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_broadcast()
        || address.is_multicast()
        || address.is_unspecified()
        || octets[0] == 0
        || (octets[0] == 100 && (octets[1] & 0xc0) == 64)
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
        || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
        || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
        || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
        || octets[0] >= 240
}
