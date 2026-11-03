# THX-01, self-hosted (for Bloo AI)

THX-01 is **open source**: Apache 2.0, weights on Hugging Face, published on PyPI as `thx01`.
There is no signup and no cloud key required. That earlier "HAL-X console" is their enterprise
gateway; the model itself is downloadable.

| | |
|---|---|
| Weights | `doofz/THX-01` (HF), F16 safetensors, ~645 MB |
| Params | 322M (307M mmBERT-base encoder + 15M decision head) |
| Runtime | Python ≥ 3.10, `torch`, `transformers` |
| REST server | `python -m thx01.server --port 8095` |
| License | Apache 2.0 |
| Port in Bloo | **27812** (host) → 8095 (container) |

---

## Option 1 — test it in the browser (2 minutes, nothing installed)

<https://huggingface.co/spaces/doofz/THX-01-demo>

Paste a campus question, add a `choice` question with your categories, and watch the
probabilities. This is the fastest way to see whether it is good enough for your documents.

---

## Option 2 — test it on your laptop in Python (30 minutes)

This is a *research* step. It is not part of the Rust product.

```powershell
cd D:\BLOO
py -3.12 -m venv .venv-thx01
.\.venv-thx01\Scripts\Activate.ps1

# CPU-only torch: ~200 MB instead of the ~2.5 GB CUDA wheel
pip install torch --index-url https://download.pytorch.org/whl/cpu

pip install "thx01[server]"
```

Then either call it as a library:

```python
import thx01
agent = thx01.load("doofz/THX-01", device="cpu")

doc = ("Hostel fee for first year students is Rs 48,000 per semester. "
       "Refunds are pro-rata and require a written application to the warden.")

print(agent.system_one(doc, {
    "topic": {"type": "choice", "question": "Which campus area is this?",
              "criteria": {"fees": "tuition, hostel fee, refund, payment",
                           "hostel": "rooms, wardens, mess, rules",
                           "exams": "timetable, results, revaluation"}},
    "fee":   {"type": "number", "question": "What is the hostel fee, in rupees?"},
    "quote": {"type": "excerpt", "question": "What does a refund require?"},
}))
```

…or run the REST server and use it exactly like the cloud API:

```powershell
$env:THX01_MODEL  = "doofz/THX-01"
$env:THX01_DEVICE = "cpu"
$env:THX01_API_KEY = "bloo-local"
python -m thx01.server --host 127.0.0.1 --port 8095
```

First start downloads ~645 MB of weights into `%USERPROFILE%\.cache\huggingface`.

---

## Option 3 — run it as a Bloo sidecar (what the demo uses)

> **On this laptop, prefer Option 2b below.** Docker Desktop can wedge while loading a
> 322M fp32 model into a WSL2 VM; running the same server as a host process avoids the
> duplicate memory and the failure mode entirely.

```powershell
docker compose -p blooai -f docker-compose.yml --profile thx01 up -d --build thx01
```

Then point Bloo at it in `.env`:

```dotenv
HALX_BASE_URL=http://127.0.0.1:27812
HALX_API_KEY=bloo-local
BLOO_DECIDER=thx
```

Restart Bloo. `/health` should now report `"decider":"thx-01"`.

Verify from the host:

```powershell
scripts\test-thx01.ps1
```

The service is capped at 3 GB (`mem_limit`) so a bad load cannot take the whole VM down.

---

## Option 2b — host process (recommended while developing)

Bloo already runs on the host in dev, so there is no reason to put torch inside Docker.

```powershell
scripts\start-thx01.ps1
```

It creates `.venv-thx01` once, installs CPU-only torch, sets `OMP_NUM_THREADS` so the laptop
stays responsive, and serves on `127.0.0.1:8095`. Then set:

```dotenv
HALX_BASE_URL=http://127.0.0.1:8095
HALX_API_KEY=bloo-local
BLOO_DECIDER=thx
```

Verify: `scripts\test-thx01.ps1 -BaseUrl http://127.0.0.1:8095`

Use Docker only for the packaged demo build.

---

## Speed and memory — read this before planning around it

The "10 ms" headline is **on one GPU**. On a laptop CPU the library itself warns it is
"roughly 10-15x slower, ~200-500 ms rather than ~35 ms", and that is per forward pass.

`number`, `excerpt` and `cite` are **additional service-layer passes**, not free. One question
with citations can be 2-4 forward passes.

| Workload | GPU | Laptop CPU |
|---|---|---|
| route (1 choice question) | ~10 ms | **96 ms model / 230 ms wall** |
| rerank 40 chunks (batch) | ~40 ms | not measured; expect seconds |
| citation on 1 chunk (excerpt + cite) | ~20 ms | ~2–3 s |
| choice + number + cite + excerpt | ~50 ms | **4.99 s** |

Measured on this project's laptop, `doofz/THX-01` fp32 on CPU, 45-word document.

**Consequences for Bloo:**

1. **Do not use THX-01 as the reranker on CPU.** `bge-reranker-base` via `fastembed` does 40
   chunks in ~30 ms on the same laptop. Keep the `Reranker` trait, default to the cross-encoder,
   use THX-01 rerank only when a GPU is present.
2. **Use THX-01 where a single pass buys a lot:** query routing (1 call) and verbatim citation
   extraction (1 call on the top chunk).
3. **Best use is offline.** At ingest time latency is irrelevant, so THX-01 can tag every
   document (`category`, `doc_type`), pull out stated numbers and dates, and pre-draft FAQ
   candidates. Batch mode is ~1 ms/decision on GPU and still cheap on CPU.

Keep `BLOO_DECIDER=local` as the fallback so a laptop without the sidecar still runs.

---

## Wiring it into Rust

Bloop's `ThxDecider` speaks `POST /v1/systemone` and `POST /v1/systemone/batch`. The local
server implements both, with the same request/response shape as the cloud API. So
**switching from cloud to self-hosted is two environment variables, zero code changes.**

Auth is optional on the local server: if `THX01_API_KEY` is unset, no bearer token is required.
We set one anyway so the Rust client path is identical to production.

---

## Offline / demo day

The weights live in the Docker volume `blooai_thx01models`. Warm it once:

```powershell
docker compose -p blooai -f docker-compose.yml --profile thx01 up -d thx01
# wait for "Application startup complete", then it is cached forever
```

For a fully offline viva, add to the service environment:

```yaml
HF_HUB_OFFLINE: "1"
TRANSFORMERS_OFFLINE: "1"
```

---

## Why not pure Rust

The model is PyTorch. Three ways to keep Rust at the centre:

| Option | Cost | Verdict |
|---|---|---|
| **Sidecar** (this folder) | +1 container, ~2 GB RAM, Python at runtime | **Recommended** — works today |
| **ONNX export + `ort`** | Export the encoder, reimplement `type_emb` / `scorer` / `act_head` / marker positions / temperature buckets in Rust; ~1-2 weeks, high risk | Stretch goal only |
| **Skip THX-01** | None | Already implemented as `BLOO_DECIDER=local` |

The sidecar is a deliberate, documented exception to "Rust only": one pinned Python process
serving a 322M research model over HTTP. Everything the user sees and everything in the
pipeline is still Rust.

If you want the ONNX path later, it slots in behind the same `Decider` trait — the Rust code
does not change.
