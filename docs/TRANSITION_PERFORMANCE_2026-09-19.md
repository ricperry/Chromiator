# Transition workload evidence, 2026-09-19

Command:

```sh
cargo test --release --test transition_performance -- --ignored --nocapture --test-threads=1
```

Both opt-in tests passed. Each timing is the median of three measured runs after
one reference render. Repeated images were compared exactly. These CPU release
measurements include recipe compilation/contact construction and rendering, but
exclude GTK, decoding and export encoding. All fixtures use Perceptual Source
matching, deterministic three-dimensional sites, and Influence values -2 through 2.
Soft width is 50%. Different row sizes are different workloads, not a scaling
curve for a fixed image.

| Sites | Image | Blend space | Hard ms | Multisite ms | Shared borders ms |
| --- | --- | --- | --- | --- | --- |
| 5 | 1024x1024 | Oklab | 74.135 | 153.139 | 192.141 |
| 5 | 1024x1024 | Linear RGB | 73.027 | 148.732 | 190.064 |
| 16 | 512x512 | Oklab | 25.208 | 51.932 | 88.280 |
| 16 | 512x512 | Linear RGB | 25.296 | 51.389 | 88.876 |
| 64 | 256x256 | Oklab | 15.510 | 29.311 | 473.376 |
| 64 | 256x256 | Linear RGB | 16.095 | 29.379 | 473.462 |

For a 64-site 512x512 workload, changing the current generation from the first
render-progress callback caused cancellation without publishing pixels. Time
from cancellation request to return: Multisite 2.055 ms; Shared borders 4.021 ms.
This tests cancellation during row rendering, not during contact construction.

The 64-site cost makes contact-graph caching a potential future optimization;
no speedup or acceleration claim is made. Do not change geometry to a nearest-two
shortcut to reduce that cost.

These results precede the pending coplanar-contact correction and JSON precision
decision recorded in `TRANSITION_VERIFICATION.md`. Repeat the affected checks
after corrections; this report is not final feature acceptance.
> Historical pre-correction measurements follow. For the final post-correction
> timing table and cancellation results, use `TRANSITION_SOLVER_CORRECTION.md`.
> Solver correction and implementation verification are now complete.
