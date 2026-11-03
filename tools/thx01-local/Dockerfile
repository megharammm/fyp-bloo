# THX-01 REST server, CPU-only.
#
# CPU torch first: the default Windows/Linux wheel bundles CUDA and is ~2.5 GB.
# From the PyTorch CPU index it is ~200 MB, which matters in a 7-week project.

FROM python:3.12-slim-bookworm

ENV PYTHONUNBUFFERED=1 \
    PIP_NO_CACHE_DIR=1 \
    HF_HOME=/models \
    THX01_MODEL=doofz/THX-01 \
    THX01_DEVICE=cpu

RUN apt-get update \
 && apt-get install -y --no-install-recommends curl ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app

RUN pip install --index-url https://download.pytorch.org/whl/cpu torch
RUN pip install "thx01[server]"

EXPOSE 8095

HEALTHCHECK --interval=15s --timeout=5s --start-period=120s --retries=10 \
  CMD curl -fsS http://127.0.0.1:8095/health || exit 1

CMD ["python", "-m", "thx01.server", "--host", "0.0.0.0", "--port", "8095"]
