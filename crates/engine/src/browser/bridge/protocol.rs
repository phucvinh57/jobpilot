//! The messages of the browser bridge (`extension/PROTOCOL.md`, protocol 1). Every frame is one JSON
//! text message with a `type`; unknown types are ignored by both sides.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The protocol version this app speaks.
pub const PROTOCOL: u32 = 1;

/// A string whose value never appears in a `Debug` dump (and so never in a log).
macro_rules! hidden {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!(stringify!($name), "(***)"))
            }
        }
    };
}

hidden!(
    /// A pairing token (sent once, in `paired`).
    Token
);
hidden!(
    /// A handshake nonce (64 hex characters).
    Nonce
);
hidden!(
    /// A handshake proof: hex HMAC-SHA256 keyed by the token (see [`super::pairing`]).
    Proof
);

/// Extension -> app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Incoming {
    /// The first message of a connection.
    Hello(Hello),
    /// The extension's answer to a [`Outgoing::Challenge`].
    Auth { proof: Proof },
    /// The user pressed Pair in the extension's popup for this code.
    Consent { code: String },
    Ping,
    /// The answer to a call.
    #[serde(rename = "result")]
    CallResult {
        id: u64,
        #[serde(default)]
        result: Value,
    },
    /// A call that failed.
    #[serde(rename = "error")]
    CallError { id: u64, error: ErrorBody },
    /// Something happened in the browser; see [`Event::parse`].
    Event {
        event: String,
        #[serde(default)]
        params: Value,
    },
    /// A type this app does not know (ignored).
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub protocol: u32,
    /// The extension's version.
    #[serde(default)]
    pub extension: String,
    /// `navigator.userAgent`.
    #[serde(default)]
    pub browser: String,
    #[serde(default)]
    pub incognito_allowed: bool,
    /// The extension holds a token (it never sends it).
    #[serde(default)]
    pub paired: bool,
    /// The extension's nonce, for the app's proof in a [`Outgoing::Challenge`].
    #[serde(default)]
    pub nonce: Option<Nonce>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: String,
    #[serde(default)]
    pub message: String,
}

/// App -> extension.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Outgoing {
    Welcome,
    /// The app's proof for the extension's nonce, and the app's nonce to answer with `auth`.
    Challenge { proof: Proof, nonce: Nonce },
    Pair { code: String },
    Paired { token: Token },
    Rejected { reason: String },
    Forget,
    Pong,
    Call { id: u64, method: String, params: Value },
}

/// The `reason` of a [`Outgoing::Rejected`].
pub mod reason {
    pub const PROTOCOL: &str = "protocol";
    pub const DECLINED: &str = "declined";
    pub const TIMEOUT: &str = "timeout";
}

/// The error `code`s of a failed call.
pub mod code {
    pub const NOT_OWNED: &str = "not_owned";
    pub const NOT_ALLOWED: &str = "not_allowed";
    pub const INCOGNITO_NOT_ALLOWED: &str = "incognito_not_allowed";
    pub const BAD_PARAMS: &str = "bad_params";
    pub const CDP: &str = "cdp";
    pub const INTERNAL: &str = "internal";
}

/// An event the app acts on (the `event` + `params` of an [`Incoming::Event`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Event {
    TabClosed { tab: i64 },
    Detached {
        tab: i64,
        #[serde(default)]
        reason: String,
    },
    #[serde(rename_all = "camelCase")]
    RequestPaused {
        tab: i64,
        #[serde(default)]
        session: Option<String>,
        request_id: String,
        url: String,
    },
    Dialog {
        tab: i64,
        #[serde(default)]
        kind: String,
        #[serde(default)]
        message: String,
    },
}

impl Event {
    /// The event named `event` with `params`; `None` for an unknown name or params that do not fit.
    pub fn parse(event: &str, params: &Value) -> Option<Event> {
        let known = matches!(event, "tabClosed" | "detached" | "requestPaused" | "dialog");
        if !known {
            return None;
        }
        let mut tagged = serde_json::Map::new();
        tagged.insert(event.to_string(), params.clone());
        serde_json::from_value(Value::Object(tagged)).ok()
    }
}

/// `tab.open`'s result.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Opened {
    pub tab: i64,
    #[serde(default)]
    pub window: i64,
}

/// `tab.info`'s result.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct TabInfo {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub loading: bool,
}

/// `cookies.domains`'s result.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct CookieDomains {
    #[serde(default)]
    pub domains: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn incoming(raw: &str) -> Incoming {
        serde_json::from_str(raw).unwrap()
    }

    #[test]
    fn a_hello_from_the_extension_parses_and_hides_its_nonce() {
        let raw = r#"{"type":"hello","protocol":1,"extension":"0.1.0","browser":"Mozilla/5.0 Chrome/153.0.0.0","incognitoAllowed":true,"paired":true,"nonce":"ab12"}"#;
        let Incoming::Hello(hello) = incoming(raw) else { panic!("not a hello") };
        assert_eq!(hello.protocol, 1);
        assert_eq!(hello.extension, "0.1.0");
        assert!(hello.incognito_allowed && hello.paired);
        assert_eq!(hello.nonce, Some(Nonce("ab12".into())));
        assert!(!format!("{hello:?}").contains("ab12"), "the nonce never shows in Debug");

        let Incoming::Hello(first) = incoming(r#"{"type":"hello","protocol":1,"extension":"0.1.0","browser":"x","incognitoAllowed":false,"paired":false,"nonce":"cd"}"#) else {
            panic!()
        };
        assert!(!first.paired && !first.incognito_allowed);
        let Incoming::Hello(bare) = incoming(r#"{"type":"hello","protocol":1}"#) else { panic!() };
        assert!(!bare.paired && bare.nonce.is_none());
    }

    #[test]
    fn auth_and_consent_parse_and_hide_the_proof() {
        let auth = incoming(r#"{"type":"auth","proof":"feed"}"#);
        assert_eq!(auth, Incoming::Auth { proof: Proof("feed".into()) });
        assert!(!format!("{auth:?}").contains("feed"));
        assert_eq!(incoming(r#"{"type":"consent","code":"123456"}"#), Incoming::Consent { code: "123456".into() });
    }

    #[test]
    fn results_errors_pings_and_unknown_types_parse() {
        assert_eq!(incoming(r#"{"type":"ping"}"#), Incoming::Ping);
        assert_eq!(incoming(r#"{"type":"result","id":7,"result":{"tab":3,"window":1}}"#), Incoming::CallResult { id: 7, result: json!({"tab":3,"window":1}) });
        assert_eq!(
            incoming(r#"{"type":"error","id":8,"error":{"code":"cdp","message":"Cannot navigate"}}"#),
            Incoming::CallError { id: 8, error: ErrorBody { code: "cdp".into(), message: "Cannot navigate".into() } }
        );
        assert_eq!(incoming(r#"{"type":"somethingNew","x":1}"#), Incoming::Unknown);
        assert!(serde_json::from_str::<Incoming>(r#"{"no":"type"}"#).is_err());
    }

    #[test]
    fn events_parse_by_name() {
        let Incoming::Event { event, params } = incoming(r#"{"type":"event","event":"requestPaused","params":{"tab":4,"session":"S1","requestId":"interception-1","url":"http://10.0.0.1/"}}"#) else {
            panic!()
        };
        assert_eq!(
            Event::parse(&event, &params),
            Some(Event::RequestPaused { tab: 4, session: Some("S1".into()), request_id: "interception-1".into(), url: "http://10.0.0.1/".into() })
        );
        assert_eq!(
            Event::parse("requestPaused", &json!({"tab":4,"requestId":"r","url":"https://a.test/"})),
            Some(Event::RequestPaused { tab: 4, session: None, request_id: "r".into(), url: "https://a.test/".into() })
        );
        assert_eq!(Event::parse("tabClosed", &json!({"tab": 9})), Some(Event::TabClosed { tab: 9 }));
        assert_eq!(Event::parse("detached", &json!({"tab": 9, "reason": "canceled_by_user"})), Some(Event::Detached { tab: 9, reason: "canceled_by_user".into() }));
        assert_eq!(
            Event::parse("dialog", &json!({"tab": 9, "kind": "alert", "message": "hi"})),
            Some(Event::Dialog { tab: 9, kind: "alert".into(), message: "hi".into() })
        );
        assert_eq!(Event::parse("somethingNew", &json!({"tab": 9})), None);
        assert_eq!(Event::parse("tabClosed", &json!({"nope": 9})), None);
    }

    #[test]
    fn outgoing_messages_have_the_documented_shape() {
        let out = |m: Outgoing| serde_json::to_value(m).unwrap();
        assert_eq!(out(Outgoing::Welcome), json!({"type":"welcome"}));
        assert_eq!(out(Outgoing::Pair { code: "123456".into() }), json!({"type":"pair","code":"123456"}));
        let challenge = Outgoing::Challenge { proof: Proof("aa".into()), nonce: Nonce("bb".into()) };
        assert!(!format!("{challenge:?}").contains("aa") && !format!("{challenge:?}").contains("bb"));
        assert_eq!(out(challenge), json!({"type":"challenge","proof":"aa","nonce":"bb"}));
        assert_eq!(out(Outgoing::Paired { token: Token("ff".into()) }), json!({"type":"paired","token":"ff"}));
        assert_eq!(out(Outgoing::Rejected { reason: reason::PROTOCOL.into() }), json!({"type":"rejected","reason":"protocol"}));
        assert_eq!(out(Outgoing::Forget), json!({"type":"forget"}));
        assert_eq!(out(Outgoing::Pong), json!({"type":"pong"}));
        assert_eq!(
            out(Outgoing::Call { id: 1, method: "tab.open".into(), params: json!({"url":"about:blank"}) }),
            json!({"type":"call","id":1,"method":"tab.open","params":{"url":"about:blank"}})
        );
    }

    #[test]
    fn every_message_round_trips() {
        let messages = [
            Outgoing::Welcome,
            Outgoing::Pair { code: "000042".into() },
            Outgoing::Challenge { proof: Proof("c".repeat(64)), nonce: Nonce("d".repeat(64)) },
            Outgoing::Paired { token: Token("a".repeat(64)) },
            Outgoing::Rejected { reason: reason::DECLINED.into() },
            Outgoing::Forget,
            Outgoing::Pong,
            Outgoing::Call { id: u64::MAX, method: "cdp.send".into(), params: json!({"tab": 1, "method": "Runtime.evaluate", "params": {}}) },
        ];
        for m in messages {
            let text = serde_json::to_string(&m).unwrap();
            assert_eq!(serde_json::from_str::<Outgoing>(&text).unwrap(), m, "{text}");
        }
        let incoming = [
            Incoming::Hello(Hello {
                protocol: 1,
                extension: "0.1.0".into(),
                browser: "ua".into(),
                incognito_allowed: true,
                paired: true,
                nonce: Some(Nonce("e".repeat(64))),
            }),
            Incoming::Auth { proof: Proof("f".repeat(64)) },
            Incoming::Consent { code: "000042".into() },
            Incoming::Ping,
            Incoming::CallResult { id: 2, result: json!({}) },
            Incoming::CallError { id: 3, error: ErrorBody { code: code::NOT_OWNED.into(), message: "gone".into() } },
            Incoming::Event { event: "tabClosed".into(), params: json!({"tab": 1}) },
        ];
        for m in incoming {
            let text = serde_json::to_string(&m).unwrap();
            assert_eq!(serde_json::from_str::<Incoming>(&text).unwrap(), m, "{text}");
        }
    }

    #[test]
    fn call_results_parse() {
        assert_eq!(serde_json::from_value::<Opened>(json!({"tab": 12, "window": 3})).unwrap(), Opened { tab: 12, window: 3 });
        let info: TabInfo = serde_json::from_value(json!({"url": "https://a.test/", "title": "A", "loading": false})).unwrap();
        assert_eq!(info.title, "A");
        let domains: CookieDomains = serde_json::from_value(json!({"domains": ["a.test", "b.a.test"]})).unwrap();
        assert_eq!(domains.domains, ["a.test", "b.a.test"]);
    }
}
