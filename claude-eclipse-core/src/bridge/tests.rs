use super::*;

fn read_exact_str(stream: &mut TcpStream, len: usize) -> String {
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).expect("read");
    String::from_utf8(buf).expect("utf8")
}

/// One combined test (the relay is a process-global singleton): a peer with
/// the wrong token is rejected, authenticated peers get wired through in
/// both directions, and relay_stop tears everything down.
#[test]
fn relay_rejects_bad_token_then_forwards_both_ways() {
    let token = "test-secret-token";
    let (port_a, port_b) =
        relay_start(47610, 47690, token).expect("two free ports in test range");
    assert!(relay_is_running());
    assert_ne!(port_a, port_b);

    // Unauthenticated peer: dropped after its bogus first line.
    {
        let mut bad = TcpStream::connect(("127.0.0.1", port_a)).unwrap();
        bad.write_all(b"wrong-token\n").unwrap();
        bad.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut one = [0u8; 1];
        assert!(
            matches!(bad.read(&mut one), Ok(0) | Err(_)),
            "unauthenticated peer must be disconnected"
        );
    }

    // Authenticated peers on both ports get wired through.
    let mut a = TcpStream::connect(("127.0.0.1", port_a)).unwrap();
    a.write_all(format!("{}\n", token).as_bytes()).unwrap();
    let mut b = TcpStream::connect(("127.0.0.1", port_b)).unwrap();
    b.write_all(format!("{}\n", token).as_bytes()).unwrap();
    a.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    b.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

    a.write_all(b"CHAT:onText:hello\n").unwrap();
    assert_eq!(read_exact_str(&mut b, 18), "CHAT:onText:hello\n");
    b.write_all(b"pong\n").unwrap();
    assert_eq!(read_exact_str(&mut a, 5), "pong\n");

    // A disconnect must end only this pairing: the relay keeps the SAME two ports
    // and accepts a fresh pair, rather than dying with the first hang-up.
    drop(a);
    drop(b);
    assert!(relay_is_running(), "a peer disconnect must not end the relay");

    let (port_a2, port_b2) =
        relay_start(47610, 47690, token).expect("relay is still up on its ports");
    assert_eq!(
        (port_a2, port_b2),
        (port_a, port_b),
        "the relay must not rebind after a disconnect"
    );

    let mut a2 = TcpStream::connect(("127.0.0.1", port_a)).unwrap();
    a2.write_all(format!("{}\n", token).as_bytes()).unwrap();
    let mut b2 = TcpStream::connect(("127.0.0.1", port_b)).unwrap();
    b2.write_all(format!("{}\n", token).as_bytes()).unwrap();
    a2.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    b2.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

    a2.write_all(b"second\n").unwrap();
    assert_eq!(read_exact_str(&mut b2, 7), "second\n");
    b2.write_all(b"back\n").unwrap();
    assert_eq!(read_exact_str(&mut a2, 5), "back\n");

    // Stop tears down whichever pairing is live at the time — here, the second one.
    relay_stop();
    assert!(!relay_is_running());
    let mut one = [0u8; 1];
    assert!(
        matches!(a2.read(&mut one), Ok(0) | Err(_)),
        "peers are torn down on stop"
    );
}
