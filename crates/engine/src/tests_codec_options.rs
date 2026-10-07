//! M13.4: HEVC / AV1 output modules and the WebM codec / Opus options over the command API.

use effectcraft_project::render_queue::{Channels, CodecProfile, OpusApplication, OutputFormat, RateControlMode, VideoCodecOptions, WebmVideoCodec};
use serde_json::json;

use crate::Session;

fn session() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

#[test]
fn hevc_and_av1_output_module_options() {
    let mut s = session();
    let a = s
        .execute(
            "renderQueue.add",
            json!({"format": "hevc", "profile": "main10", "level": "4.1", "rateControl": "quality", "quality": 85, "keyframeInterval": 30}),
        )
        .unwrap();
    assert!(a["outputPath"].as_str().unwrap().ends_with(".mp4"), "{a}");
    let om = &s.project.render_queue[0].output;
    assert_eq!(om.format, OutputFormat::Hevc);
    assert_eq!(om.codec, VideoCodecOptions { profile: CodecProfile::Main10, level: Some(41), rate_control: RateControlMode::Quality, quality: 85 });
    assert_eq!(om.keyframe_interval, 30);
    assert_eq!(a["output"]["codec"]["profile"], "Main10", "{a}");
    let id = a["item"].clone();
    s.execute("renderQueue.setOutputModule", json!({"item": id, "format": "av1", "level": null, "rateControl": "bitrate", "bitrate": 3000})).unwrap();
    let om = &s.project.render_queue[0].output;
    assert_eq!((om.format, om.codec.level, om.codec.rate_control, om.bitrate_kbps), (OutputFormat::Av1, None, RateControlMode::Bitrate, 3000));
    assert!(s.execute("renderQueue.setOutputModule", json!({"item": id, "channels": "rgba"})).is_err(), "AV1 MP4 has no alpha");
    assert!(s.execute("renderQueue.setOutputModule", json!({"item": id, "profile": "high"})).is_err());
    assert!(s.execute("renderQueue.setOutputModule", json!({"item": id, "level": "0.5"})).is_err());
    assert!(s.execute("renderQueue.setOutputModule", json!({"item": id, "rateControl": "cbr2"})).is_err());

    // WebM: VP9 with alpha, then AV1 (alpha dropped) with voice-tuned Opus.
    s.execute("renderQueue.setOutputModule", json!({"item": id, "format": "webm", "channels": "rgba"})).unwrap();
    assert_eq!(s.project.render_queue[0].output.channels, Channels::Rgba);
    s.execute("renderQueue.setOutputModule", json!({"item": id, "webmCodec": "av1", "audioBitrate": 24, "opusApplication": "voice"})).unwrap();
    let om = &s.project.render_queue[0].output;
    assert_eq!((om.webm_codec, om.channels, om.opus_bitrate_kbps, om.opus_application), (WebmVideoCodec::Av1, Channels::Rgb, 24, OpusApplication::Voip));
    assert!(s.execute("renderQueue.setOutputModule", json!({"item": id, "webmCodec": "theora"})).is_err());

    // The formats query lists the new formats and option values.
    let f = s.execute("renderQueue.formats", json!({})).unwrap();
    let ids: Vec<&str> = f["formats"].as_array().unwrap().iter().filter_map(|v| v["id"].as_str()).collect();
    assert!(ids.contains(&"Hevc") && ids.contains(&"Av1"), "{ids:?}");
    assert_eq!(f["codecProfiles"], json!(["Main", "Main 10"]));
    assert!(f["hevcLevels"].as_array().unwrap().contains(&json!("4.1")));
    assert_eq!(f["webmCodecs"], json!(["vp9", "av1"]));
}

/// An explicit format wins over the output file's extension, which follows it (#154:
/// `effectcraft-cli render --format hevc --out x.mp4` wrote H.264). Without a format, the extension
/// still picks one.
#[test]
fn an_explicit_format_wins_over_the_output_extension() {
    let mut s = session();
    let cases = [
        ("hevc", "/out/x.mp4", OutputFormat::Hevc, "/out/x.mp4"),
        ("av1", "/out/x.mp4", OutputFormat::Av1, "/out/x.mp4"),
        ("hevc", "/out/x.mov", OutputFormat::Hevc, "/out/x.mp4"),
        ("av1", "/out/x.webm", OutputFormat::Av1, "/out/x.mp4"),
        ("prores", "/out/x.mp4", OutputFormat::ProRes, "/out/x.mov"),
    ];
    for (k, (format, out, want, path)) in cases.into_iter().enumerate() {
        s.execute("renderQueue.add", json!({"format": format, "output": out})).unwrap();
        let om = &s.project.render_queue[k].output;
        assert_eq!((om.format, om.output.as_str()), (want, path), "{format} → {out}");
        // A second output module of the item behaves the same.
        s.execute("render.addOutputModule", json!({"index": k + 1, "format": format, "output": out})).unwrap();
        let extra = &s.project.render_queue[k].extra_outputs[0];
        assert_eq!((extra.format, extra.output.as_str()), (want, path), "addOutputModule {format} → {out}");
    }
    // No format: the extension picks it, as before. A format the extension agrees with keeps
    // the name as given (the CLI passes the extension as the format).
    s.execute("renderQueue.add", json!({"output": "/out/y.mov"})).unwrap();
    assert_eq!(s.project.render_queue.last().unwrap().output.format, OutputFormat::ProRes);
    s.execute("renderQueue.add", json!({"format": "png", "output": "/out/still.png"})).unwrap();
    let om = &s.project.render_queue.last().unwrap().output;
    assert_eq!((om.format, om.output.as_str()), (OutputFormat::PngSequence, "/out/still.png"));
}

/// A `comp` that names no composition says so, rather than "no active composition" (#155).
#[test]
fn an_unknown_comp_is_named_in_the_error() {
    let mut s = session();
    for (cmd, p) in [
        ("renderQueue.add", json!({"comp": "Nope", "output": "/out/x.mp4"})),
        ("layer.newSolid", json!({"comp": "Nope"})),
        ("comp.settings", json!({"comp": 99_999, "width": 10})),
    ] {
        let e = s.execute(cmd, p.clone()).unwrap_err().to_string();
        let name = p["comp"].to_string();
        assert!(e.contains(&format!("no composition {name}")), "{cmd}: {e}");
    }
}
