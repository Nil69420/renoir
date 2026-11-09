# Migration Guides

These guides answer the question an evaluator asks first: *"what do I
gain and lose switching from X to Renoir?"* They state the real gaps
plainly — including things Renoir does not do yet — based on this
project's own research and measurements, not marketing.

| From | What you gain | What you give up |
|---|---|---|
| [iceoryx2](iceoryx2.md) | variable-size payloads; Python bindings; smaller surface | ~1.8x latency in our benchmark |
| [eCAL](ecal.md) | simpler deployment; fewer moving parts | recording/replay + monitor tooling |
| [Zenoh](zenoh.md) | same-host latency without routing overhead | multi-host networking |
