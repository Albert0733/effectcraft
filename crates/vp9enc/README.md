# effectcraft-vp9enc

A small VP9 encoder for WebM export: every frame is a key frame (profile 0, 8-bit 4:2:0,
BT.709), coded with 8×8 intra blocks (DC / V / H / TM prediction chosen per block), 4×4
DCT/ADST transforms with default probabilities, or the Walsh–Hadamard transform at quality 100
(mathematically lossless).

Written from the **VP9 Bitstream & Decoding Process Specification, version 0.6 (31 March
2016)**: the encoder emits the uncompressed and compressed headers (§6.2, §6.3), partition, mode
info and token syntax (§6.4) and reconstructs with the specification's intra prediction (§8.5.1)
and integer inverse transforms (§8.7) so it stays in step with any conformant decoder. The
boolean encoder is the inverse of the §9.2 decoder in the form given in RFC 6386 §7.3. Spec
tables (§10) were transcribed from FilmCraft's first-party decoder `filmcraft-vp9` (MIT OR
Apache-2.0), which is also the decoder the tests round-trip through; ffmpeg is used only as an
external test oracle. No libvpx or other third-party encoder/decoder source was consulted.
