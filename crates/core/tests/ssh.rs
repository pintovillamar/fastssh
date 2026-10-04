//! SSH client tests against a small in-process SSH server.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use fastssh_core::Store;
use fastssh_core::ssh::{self, HostKeyQuestion, KeyError};
use fastssh_core::store::{AuthKind, ConnectionDetails, SavedConnection};
use russh::keys::{PublicKey, decode_secret_key};
use russh::server::{self, Auth, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

// Keys made only for these tests; they protect nothing.
const HOST_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACBdIGawof6LXHmOA6UF0/q8kf2lR1ZpyexLpM12lEkm/wAAAIhxy1D+cctQ
/gAAAAtzc2gtZWQyNTUxOQAAACBdIGawof6LXHmOA6UF0/q8kf2lR1ZpyexLpM12lEkm/w
AAAEB6eeUob5D3tXTADBq/t/EWs2c6jahf/7oGa8nTW/GRJV0gZrCh/otceY4DpQXT+ryR
/aVHVmnJ7EukzXaUSSb/AAAABHRlc3QB
-----END OPENSSH PRIVATE KEY-----
";
const CLIENT_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACBBstVYx9hkigh+/PN2ZJSmMHXiPnnZcIOEmV9/N9P5cQAAAIjPQY3/z0GN
/wAAAAtzc2gtZWQyNTUxOQAAACBBstVYx9hkigh+/PN2ZJSmMHXiPnnZcIOEmV9/N9P5cQ
AAAEBX0YTwkYMJ/gIOrZ5XonVnAOONY5TZKnLrhiQI3xWYuUGy1VjH2GSKCH7883ZklKYw
deI+edlwg4SZX3830/lxAAAABHRlc3QB
-----END OPENSSH PRIVATE KEY-----
";
const OTHER_PUBLIC_KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIIkLN4xLhJGLZaNyvFw4I2QUSjMwEKLRe9GFjW+244Qs";

const USER: &str = "tester";
const PASSWORD: &str = "open sesame";

/// Accepts one user with one password or one key, and echoes whatever the
/// shell receives back as `echo:<input>`.
#[derive(Clone)]
struct TestServer {
    allowed_key: PublicKey,
}

impl server::Server for TestServer {
    type Handler = Self;

    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        self.clone()
    }
}

impl server::Handler for TestServer {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if user == USER && password == PASSWORD {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_publickey(&mut self, user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        Ok(if user == USER && key.key_data() == self.allowed_key.key_data() {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn data(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        session.data(channel, [b"echo:", data].concat())?;
        Ok(())
    }
}

struct Fixture {
    store: Store,
    user_id: i64,
    connection: SavedConnection,
}

/// Starts the test server on a free port and saves a connection to it.
async fn fixture() -> Fixture {
    let client_key = decode_secret_key(CLIENT_KEY, None).unwrap();
    let config = Arc::new(server::Config {
        keys: vec![decode_secret_key(HOST_KEY, None).unwrap()],
        auth_rejection_time: Duration::ZERO,
        ..Default::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut server = TestServer {
            allowed_key: client_key.public_key().clone(),
        };
        let _ = server.run_on_socket(config, &listener).await;
    });

    let store = Store::in_memory().unwrap();
    let user_id = store.create_user("t@example.com", None, false, 0).unwrap().id;
    let details = ConnectionDetails {
        name: "test".into(),
        host: "127.0.0.1".into(),
        port,
        username: USER.into(),
        auth: AuthKind::Password,
    };
    let connection = store.add_connection(user_id, details, None).unwrap();
    Fixture {
        store,
        user_id,
        connection,
    }
}

/// Connects, answering any host key question with `trust`. Also reports how
/// many questions were asked.
async fn connect(fixture: &Fixture, trust: bool) -> (anyhow::Result<ssh::SshClient>, usize) {
    let (tx, mut questions) = mpsc::channel::<HostKeyQuestion>(1);
    let asked = Arc::new(AtomicUsize::new(0));
    let counter = asked.clone();
    // The session keeps its end of the channel for as long as it lives, so
    // this task is stopped rather than waited for.
    let answering = tokio::spawn(async move {
        while let Some(question) = questions.recv().await {
            assert!(question.fingerprint.starts_with("SHA256:"));
            assert_eq!(question.algorithm, "ssh-ed25519");
            counter.fetch_add(1, Ordering::SeqCst);
            let _ = question.reply.send(trust);
        }
    });
    let result = ssh::connect(fixture.user_id, &fixture.connection, fixture.store.clone(), tx).await;
    answering.abort();
    (result, asked.load(Ordering::SeqCst))
}

async fn read_until(output: &mut mpsc::Receiver<Vec<u8>>, needle: &str) -> String {
    let mut seen = String::new();
    while !seen.contains(needle) {
        let chunk = tokio::time::timeout(Duration::from_secs(5), output.recv())
            .await
            .expect("output in time")
            .expect("session still open");
        seen.push_str(&String::from_utf8_lossy(&chunk));
    }
    seen
}

#[tokio::test]
async fn password_sign_in_and_shell() {
    let fixture = fixture().await;
    let (client, asked) = connect(&fixture, true).await;
    let mut client = client.unwrap();
    assert_eq!(asked, 1, "an unknown server is asked about");

    assert!(!client.auth_password("wrong".into()).await.unwrap());
    assert!(client.auth_password(PASSWORD.into()).await.unwrap());

    let (shell, mut output) = client.open_shell(80, 24).await.unwrap();
    shell.write(b"hello".to_vec()).await;
    read_until(&mut output, "echo:hello").await;
    shell.resize(120, 40).await;
    shell.write(b"again".to_vec()).await;
    read_until(&mut output, "echo:again").await;
}

#[tokio::test]
async fn key_sign_in() {
    let fixture = fixture().await;
    let mut client = connect(&fixture, true).await.0.unwrap();
    let key = ssh::decode_key(CLIENT_KEY.into(), None).await.unwrap();
    assert!(client.auth_key(key).await.unwrap());

    let mut client = connect(&fixture, true).await.0.unwrap();
    let wrong = ssh::decode_key(HOST_KEY.into(), None).await.unwrap();
    assert!(!client.auth_key(wrong).await.unwrap());
}

#[tokio::test]
async fn trusted_host_is_not_asked_about_again() {
    let fixture = fixture().await;
    assert_eq!(connect(&fixture, true).await.1, 1);
    let (client, asked) = connect(&fixture, false).await;
    assert!(client.is_ok());
    assert_eq!(asked, 0);
}

#[tokio::test]
async fn declined_host_key_stops_the_connection_and_is_not_remembered() {
    let fixture = fixture().await;
    let (client, asked) = connect(&fixture, false).await;
    assert!(client.is_err());
    assert_eq!(asked, 1);
    let details = &fixture.connection.details;
    assert_eq!(
        fixture.store.known_host(fixture.user_id, &details.host, details.port).unwrap(),
        None
    );
}

#[tokio::test]
async fn changed_host_key_is_refused_without_asking() {
    let fixture = fixture().await;
    let details = &fixture.connection.details;
    fixture
        .store
        .trust_host(fixture.user_id, &details.host, details.port, OTHER_PUBLIC_KEY)
        .unwrap();

    let (client, asked) = connect(&fixture, true).await;
    let message = format!("{:#}", client.err().expect("refused"));
    assert!(message.contains("has changed"), "{message}");
    assert_eq!(asked, 0);
}

#[tokio::test]
async fn unusable_keys_are_reported() {
    assert!(matches!(
        ssh::decode_key("not a key".into(), None).await,
        Err(KeyError::Invalid(_))
    ));
}
