//! The Web Worker driver (#265) without a browser: its two halves connected in-process.

use super::*;
use crate::scheduler::protocol;
use crate::scheduler::remote::{Client, Serve};

/// Hands `msg` to the worker's half and steps it until idle, passing every reply to the
/// app's half; returns the results delivered.
fn remote_run(client: &mut Client, serve: &mut Serve, msg: Vec<u8>) -> Vec<SolveResult> {
    serve.receive(msg).unwrap();
    let mut out = Vec::new();
    while serve.has_work() {
        if let Some(reply) = serve.step().unwrap() {
            client.receive(&reply).unwrap();
            out.extend(client.poll());
        }
    }
    out
}

fn client(preview_full: bool) -> Client {
    Client::new(Config {
        preview_full,
        ..Config::default()
    })
}

/// Pins, locks and allow/deny lists on a few cells.
fn steered() -> Steering {
    let mut st = r2_steering(CellPos::new(3, 4));
    st.set_pin(
        CellPos::new(39, 24),
        Pin {
            colours: ColourSet::from_bits(0b1010_0000_0000_0011),
            full: true,
        },
    );
    st.set_lock(CellPos::new(10, 2), true);
    st.set_allow(CellPos::new(5, 5), ColourSet::from_bits(0x00ff));
    st.set_deny(CellPos::new(6, 5), ColourSet::from_bits(0x0f00));
    st
}

#[test]
fn the_worker_protocol_round_trips_requests_and_results() {
    for (mode, n) in [(Mode::MulticolorBitmap, 2), (Mode::HiresBitmap, 1)] {
        let grid = SuperGrid::new(mode, n).unwrap();
        let mut c = client(true);
        let mut s1 = snapshot(40.0, None);
        s1.grid = grid;
        let (_, first) = c.submit(s1.clone(), SolveMode::Full);
        let r1 = remote_run(&mut c, &mut Serve::default(), first)
            .pop()
            .unwrap();

        // A steered, re-paletted full solve with a previous solution and a preview.
        let mut s2 = snapshot(40.5, Some(&r1));
        s2.grid = grid;
        s2.steering = steered();
        s2.palette = Palette::colodore(40.0, 90.0, 60.0);
        s2.options.luma_pair_bias = -0.012_345_7;
        s2.options.hysteresis = Some(0.97);
        s2.options.divergences.pair_bias = 0.1;
        s2.options.divergences.mix_levels = Some(3);
        let (ticket, msg) = c.submit(s2.clone(), SolveMode::Full);
        let req = protocol::decode_solve(&msg).unwrap();
        assert_eq!(req.ticket, ticket);
        let mut direct = Direct::new();
        direct.compositor.set_grid(grid);
        direct.update(&s1);
        direct.update(&s2);
        assert_eq!(req.frame, *direct.compositor.frame());
        assert_eq!(req.steering, s2.steering);
        assert_eq!(req.palette, s2.palette);
        let options = SolveOptions {
            layer_origins: convert64_core::solve::dither::layer_origins(grid, &s2.layers),
            ..s2.options.clone()
        };
        assert_eq!(req.options, options);
        assert_eq!(
            req.preview,
            Some(CellSet::all()),
            "another palette: every cell"
        );
        assert_eq!(req.mode, SolveMode::Full);
        assert_eq!(req.previous.as_ref(), Some(&*r1.solution));

        let reply = protocol::encode_result(ticket, true, req.mode, 12.5, &r1.solution);
        let back = protocol::decode_result(&reply).unwrap();
        assert_eq!(back.ticket, ticket);
        assert!(back.provisional);
        assert_eq!(back.mode, SolveMode::Full);
        assert_eq!(back.solve_ms.to_bits(), 12.5f64.to_bits());
        assert_eq!(back.solution, *r1.solution);
    }
}

#[test]
fn the_worker_protocol_rejects_what_it_did_not_write() {
    assert!(protocol::header(b"").is_err());
    assert!(protocol::header(b"PNG\x89\x01\x01\0\0\0\0\0\0\0\0").is_err());
    let mut c = client(false);
    let (_, mut msg) = c.submit(snapshot(40.0, None), SolveMode::Full);
    assert!(protocol::decode_result(&msg).is_err(), "a request");
    msg.push(0);
    assert!(protocol::decode_solve(&msg).is_err(), "trailing bytes");
    msg.truncate(msg.len() / 2);
    assert!(protocol::decode_solve(&msg).is_err(), "truncated");
    msg[4] = protocol::VERSION + 1;
    assert!(protocol::header(&msg).is_err(), "another version");
}

/// The header carries the core's serde types as JSON. An `f32` goes through `f64` there
/// (`serde_json::Value`), so every finite one reads back bit for bit.
#[test]
fn json_header_floats_read_back_bit_for_bit() {
    for bits in (0..=u32::MAX).step_by(9_973) {
        let x = f32::from_bits(bits);
        if !x.is_finite() {
            continue;
        }
        let text = serde_json::to_vec(&serde_json::to_value(x).unwrap()).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&text).unwrap();
        let y: f32 = serde_json::from_value(value).unwrap();
        assert_eq!(y.to_bits(), bits, "{x:e}");
    }
}

#[test]
fn the_web_worker_output_is_the_cores() {
    let mut c = client(true);
    let mut serve = Serve::default();
    let mut direct = Direct::new();

    let s1 = snapshot(40.0, None);
    let (t1, msg) = c.submit(s1.clone(), SolveMode::Full);
    let got = remote_run(&mut c, &mut serve, msg);
    assert_eq!(got.len(), 1, "no previous solution, no preview");
    let r1 = &got[0];
    assert_eq!(r1.ticket, t1);
    direct.update(&s1);
    assert_eq!(*r1.solution, direct.solve(&s1, SolveMode::Full));
    assert!(!c.is_busy());

    // A drag frame: the fast path on the cells it touched.
    let s2 = snapshot(40.25, Some(r1));
    let (_, msg) = c.submit(s2.clone(), interactive());
    let r2 = remote_run(&mut c, &mut serve, msg).pop().unwrap();
    let dirty = SolveMode::Interactive {
        dirty: direct.update(&s2),
    };
    assert_eq!(r2.mode, dirty);
    assert_eq!(*r2.solution, direct.solve(&s2, dirty));

    // The release with steering: the preview, then the full search.
    let mut s3 = snapshot(41.0, Some(&r2));
    s3.steering = steered();
    let (t3, msg) = c.submit(s3.clone(), SolveMode::Full);
    let got = remote_run(&mut c, &mut serve, msg);
    assert_eq!(got.len(), 2);
    assert!(got.iter().all(|r| r.ticket == t3));
    direct.update(&s3);
    assert!(got[0].provisional);
    assert!(matches!(got[0].mode, SolveMode::Interactive { .. }));
    assert_eq!(*got[0].solution, direct.solve(&s3, got[0].mode));
    assert!(!got[1].provisional);
    assert_eq!(got[1].mode, SolveMode::Full);
    assert_eq!(*got[1].solution, direct.solve(&s3, SolveMode::Full));
    assert!(!c.is_busy());
}

/// The same submissions through the stepping driver (what the browser ran before #265)
/// and through the Web Worker driver: the same results, bit for bit.
#[test]
fn the_web_worker_matches_the_stepping_driver() {
    let mut c = client(true);
    let mut serve = Serve::default();
    let mut s = stepper(true);
    let (_, msg) = c.submit(snapshot(40.0, None), SolveMode::Full);
    let remote1 = remote_run(&mut c, &mut serve, msg).pop().unwrap();
    s.submit(snapshot(40.0, None), SolveMode::Full);
    let local1 = s.finish().unwrap();
    assert_eq!(remote1.solution, local1.solution);

    let mut previous = (remote1, local1);
    for (x, mode) in [
        (40.25, interactive()),
        (41.0, interactive()),
        (41.0, SolveMode::Full),
    ] {
        let mut snap = snapshot(x, Some(&previous.0));
        snap.steering = steered();
        let (_, msg) = c.submit(snap.clone(), mode);
        let remote = remote_run(&mut c, &mut serve, msg);
        snap.previous = Some(Arc::clone(&previous.1.solution));
        s.submit(snap, mode);
        let local = drain(&mut s);
        assert_eq!(remote.len(), local.len());
        for (r, l) in remote.iter().zip(&local) {
            assert_eq!((r.mode, r.provisional), (l.mode, l.provisional));
            assert_eq!(r.solution, l.solution);
        }
        previous = (
            remote.last().unwrap().clone(),
            local.last().unwrap().clone(),
        );
    }
}

#[test]
fn the_web_worker_drops_superseded_work_between_stages() {
    let mut c = client(true);
    let mut serve = Serve::default();
    let (_, msg) = c.submit(snapshot(40.0, None), SolveMode::Full);
    let r1 = remote_run(&mut c, &mut serve, msg).pop().unwrap();

    // A release: the worker runs its preview stage...
    let (t2, msg) = c.submit(snapshot(41.0, Some(&r1)), SolveMode::Full);
    serve.receive(msg).unwrap();
    let preview = serve.step().unwrap().unwrap();
    assert!(serve.has_work(), "the full search is next");
    // ...then a new drag arrives before the full search: the full search never runs.
    let (t3, msg) = c.submit(snapshot(42.0, Some(&r1)), interactive());
    serve.receive(msg).unwrap();
    let next = serve.step().unwrap().unwrap();
    assert_eq!(protocol::header(&next).unwrap().ticket, t3);
    assert!(!serve.has_work());
    assert_eq!(serve.step().unwrap(), None);

    // The superseded preview arriving late delivers nothing.
    assert_eq!(protocol::header(&preview).unwrap().ticket, t2);
    assert!(!c.receive(&preview).unwrap());
    assert!(c.poll().is_none());
    assert!(c.receive(&next).unwrap());
    let r3 = c.poll().unwrap();
    assert_eq!(r3.ticket, t3);
    assert!(!r3.provisional);
    assert!(!c.is_busy());
}

#[test]
fn a_request_superseded_before_it_starts_is_never_decoded() {
    let mut c = client(false);
    let mut serve = Serve::default();
    let (_, mut stale) = c.submit(snapshot(40.0, None), SolveMode::Full);
    // Cut the stale request's body short: decoding it would fail.
    stale.truncate(40);
    serve.receive(stale).unwrap();
    let (t2, msg) = c.submit(snapshot(41.0, None), SolveMode::Full);
    let got = remote_run(&mut c, &mut serve, msg);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].ticket, t2);
}

#[test]
fn a_cancel_reaches_the_web_worker() {
    let mut c = client(true);
    let mut serve = Serve::default();
    let (_, msg) = c.submit(snapshot(40.0, None), SolveMode::Full);
    let r1 = remote_run(&mut c, &mut serve, msg).pop().unwrap();

    let (_, msg) = c.submit(snapshot(41.0, Some(&r1)), SolveMode::Full);
    serve.receive(msg).unwrap();
    let preview = serve.step().unwrap().unwrap();
    let cancel = c.cancel();
    assert!(!c.is_busy());
    serve.receive(cancel).unwrap();
    assert!(!serve.has_work());
    assert_eq!(serve.step().unwrap(), None);
    // The worker says it is idle at the cancel's ticket; the page then knows it is free.
    assert_eq!(serve.ticket(), c.last_ticket());
    let idle = protocol::header(&protocol::encode_idle(serve.ticket())).unwrap();
    assert_eq!(
        (idle.kind, idle.ticket),
        (protocol::Kind::Idle, c.last_ticket())
    );
    assert!(!c.receive(&preview).unwrap());
    assert!(c.poll().is_none());
}

/// When the worker fails, the stepping driver takes over the submission in it.
#[test]
fn the_submission_in_a_failed_worker_can_move_to_the_page() {
    let mut c = client(true);
    assert!(c.take_pending().is_none());
    let snap = snapshot(42.0, None);
    let (ticket, _) = c.submit(snap.clone(), interactive());
    let (pending, mode) = c.take_pending().unwrap();
    assert_eq!(mode, interactive());
    assert_eq!(pending.layers.len(), snap.layers.len());
    assert!(!c.is_busy());

    // As `web::Remote::into_fallback` does: the stepper keeps the submission's ticket.
    let mut s = stepper(true);
    s.continue_tickets(c.last_ticket() - 1);
    assert_eq!(s.submit(pending, mode), ticket);
    let r = s.finish().unwrap();
    assert_eq!(r.ticket, ticket);
}
