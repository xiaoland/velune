//! The public endpoint boundary must produce usable request URLs for DNS and IP hosts.
use velune_ai_provider::config::{HttpEndpoint, Transport};

const CASES: [(&str, Transport, &str, u16, &str); 4] = [
    (
        "https://api.example/v1",
        Transport::Https,
        "api.example",
        443,
        "https://api.example:443/v1/responses",
    ),
    (
        "http://127.0.0.1:4242/v1",
        Transport::Http,
        "127.0.0.1",
        4242,
        "http://127.0.0.1:4242/v1/responses",
    ),
    (
        "http://[::1]:4242/v1",
        Transport::Http,
        "::1",
        4242,
        "http://[::1]:4242/v1/responses",
    ),
    (
        "https://[2001:db8::1]/v1",
        Transport::Https,
        "2001:db8::1",
        443,
        "https://[2001:db8::1]:443/v1/responses",
    ),
];

#[test]
fn parsed_endpoints_preserve_dns_ipv4_and_ipv6_request_destinations() {
    for (source, _, host, _, expected) in CASES {
        let endpoint =
            HttpEndpoint::parse(source).unwrap_or_else(|error| panic!("{source}: {error}"));
        assert_eq!(endpoint.host(), host);
        assert_eq!(endpoint.url("/responses"), expected);
    }
}

#[test]
fn structured_endpoints_bracket_ipv6_only_when_rendering_a_request_url() {
    for (_, transport, host, port, expected) in CASES {
        let endpoint = HttpEndpoint::new(transport, host, port, "/v1/").unwrap();
        assert_eq!(endpoint.host(), host);
        assert_eq!(endpoint.url("/responses"), expected);
    }
}
