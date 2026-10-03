# effectcraft-opusenc

Clean-room, pure-Rust Opus encoder for EffectCraft's WebM export audio (layer L0, `std` only,
no `unsafe`).

- **CELT-only** packets (TOC configuration 31: fullband 20 kHz, 20 ms, one frame per packet),
  48 kHz input, mono or stereo (stereo coded as dual L/R stereo), constant packet size from the
  requested bitrate; all-zero frames become 3-byte silence packets.
- Forward MDCT with the low-overlap window, band energies with inter/intra coarse prediction
  (cheaper of the two per frame), fine and final energy bits, the decoder's exact bit allocation,
  spreading and recursive band splitting, greedy PVQ search and codeword indexing.
- Not implemented: SILK/hybrid modes, transient (short-block) frames, TF changes, pitch
  post-filter, intensity stereo, dynamic allocation boosts, VBR rate control.
- `OpusEncoder::pre_skip()` is 120 samples (the MDCT overlap); `opus_head()` returns the RFC 7845
  identification header.

```rust
let mut enc = effectcraft_opusenc::OpusEncoder::new(2, 128_000);
let packet = enc.encode_float(&vec![0.0; effectcraft_opusenc::OpusEncoder::FRAME_SIZE * 2]);
```

## Specifications

- RFC 6716, *Definition of the Opus Audio Codec* (September 2012), with the RFC 8251 updates
  (October 2017) as implemented by the decoder we round-trip against.
- RFC 7845, *Ogg Encapsulation for the Opus Audio Codec* (April 2016), §5.1 `OpusHead` and
  §4.2 pre-skip.

Implemented from the RFC text. Constant tables (band edges, `eMeans`, the coarse-energy Laplace
model, the allocation table, ICDFs) and the decoder-side allocation arithmetic are mirrored from
FilmCraft's first-party, spec-derived `filmcraft-opus` decoder (MIT OR Apache-2.0), which is also
the round-trip oracle in `crates/export/tests/opus_roundtrip.rs` (ffmpeg is a second, external
oracle when installed). No third-party encoder or decoder source was consulted.
