# Preview connection ownership repair

The Python CI stall on F1c1 head `f0f4bd0` exposes a pre-existing preview
transport defect. An independent six-connection keep-alive burst on the frozen
native binary serves four clients and strands two until a served client closes.
Four clients are the sensitive positive control. This fits the stalled Cesium
manifest request; the original CI socket mapping remains an inference.

## Invariants and disposition before implementation

Each accepted HTTP/1 connection must make progress independently of other idle,
partial or persistent clients. Idle sockets must yield the executor. File bodies
must stream under socket backpressure rather than buffer whole assets. A client
disconnect or malformed request terminates that connection, not the listener.
Listener/runtime failures fail the preview process. Bind and announce the URL
only after selected roots have passed existing validation.

Retain the concrete selected-root configuration, canonical path fence, read-only
GET/HEAD policy, MIME types, no-cache responses and embedded page. Rework only
transport adaptation. Replace tiny_http: its reader TaskPool counts idle readers
without reserving them for queued clients, and current upstream has the same
algorithm. More response workers cannot fix reader admission. Remove that
dependency and its implicit per-client reader pool. Do not vendor the complete
dependency, adopt an unproven fork, write an HTTP codec, add retries, increase
browser deadlines or alter Cesium acceptance.

Use Hyper's HTTP/1 codec with one Tokio task per accepted connection and streamed
Tokio file bodies. The listener owns task admission; each connection task owns
its socket/service; each body owns its opened file. Enable only the required
HTTP/1 server/runtime/I/O features, without introducing a router/framework.
This remains a local read-only preview, not a general web hosting contract.

## Independent acceptance and limits

Keep the executed old-binary burst/release-on-close receipt. Require all burst
clients to receive correct bodies while every keep-alive remains open, and reuse
those connections. Idle/partial clients must not prevent complete requests.
Replay existing route, HEAD, traversal and concurrent file tests and the unchanged
real terrain/Cesium rendering, height, clamp and imagery acceptance. Bind new
source and binaries separately from the earlier placement evidence; source
placement/math is unchanged. A separate nonauthor review inspects the final
implementation and receipts. Hosted platform CI remains a gate.
