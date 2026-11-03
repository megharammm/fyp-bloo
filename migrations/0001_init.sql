-- Bloo AI · schema v1
-- Postgres is the source of truth. pgvector holds the derived index.
-- Safe to re-run only through sqlx migrations (never by hand against prod).

CREATE EXTENSION IF NOT EXISTS vector;
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- ─────────────────────────────────────────────────────────────
-- Sources: where knowledge comes from
-- ─────────────────────────────────────────────────────────────
CREATE TABLE sources (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    kind          text NOT NULL CHECK (kind IN ('upload', 'seed', 'url', 'crawl')),
    name          text NOT NULL,
    config        jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at    timestamptz NOT NULL DEFAULT now(),
    last_synced_at timestamptz
);

-- ─────────────────────────────────────────────────────────────
-- Documents: one row per logical document (versioned by file_hash)
-- ─────────────────────────────────────────────────────────────
CREATE TABLE documents (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    source_id        uuid REFERENCES sources(id) ON DELETE SET NULL,
    title            text NOT NULL,
    canonical_url    text,
    file_path        text,
    file_hash        text NOT NULL,
    mime             text NOT NULL,
    kind             text NOT NULL DEFAULT 'pdf'
                     CHECK (kind IN ('pdf', 'docx', 'html', 'md', 'txt')),
    category         text NOT NULL DEFAULT 'general',
    status           text NOT NULL DEFAULT 'processing'
                     CHECK (status IN ('processing', 'ready', 'published', 'archived', 'failed')),
    version          integer NOT NULL DEFAULT 1,
    page_count       integer,
    valid_until      date,
    last_verified_at date,
    error            text,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX documents_file_hash_uidx ON documents (file_hash);
CREATE INDEX documents_status_idx ON documents (status);
CREATE INDEX documents_category_idx ON documents (category);
CREATE INDEX documents_title_trgm_idx ON documents USING gin (title gin_trgm_ops);

-- ─────────────────────────────────────────────────────────────
-- Chunks: derived, rebuildable. Embedding lives here.
-- 384 dims = BAAI/bge-small-en-v1.5. Change both together.
-- ─────────────────────────────────────────────────────────────
CREATE TABLE chunks (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    document_id  uuid NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    ord          integer NOT NULL,
    text         text NOT NULL,
    section_path text NOT NULL DEFAULT '',
    page_from    integer,
    page_to      integer,
    chunk_type   text NOT NULL DEFAULT 'prose'
                 CHECK (chunk_type IN ('prose', 'table', 'faq')),
    token_count  integer,
    embedding    vector(384),
    -- lexical half of hybrid search; 'simple' keeps numbers and names intact
    tsv          tsvector GENERATED ALWAYS AS (
                     to_tsvector('simple', coalesce(section_path, '') || ' ' || text)
                 ) STORED,
    created_at   timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX chunks_document_idx ON chunks (document_id, ord);
CREATE INDEX chunks_embedding_idx ON chunks
    USING hnsw (embedding vector_cosine_ops) WITH (m = 16, ef_construction = 64);
CREATE INDEX chunks_tsv_idx ON chunks USING gin (tsv);

-- ─────────────────────────────────────────────────────────────
-- Jobs: Postgres-backed queue (SELECT … FOR UPDATE SKIP LOCKED).
-- No Redis on purpose — one less daemon, ~40 MB saved.
-- ─────────────────────────────────────────────────────────────
CREATE TABLE jobs (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    kind         text NOT NULL CHECK (kind IN ('ingest', 'crawl', 'embed', 'sync')),
    document_id  uuid REFERENCES documents(id) ON DELETE CASCADE,
    state        text NOT NULL DEFAULT 'queued'
                 CHECK (state IN ('queued', 'running', 'done', 'failed')),
    progress     integer NOT NULL DEFAULT 0 CHECK (progress BETWEEN 0 AND 100),
    stage        text,
    attempts     integer NOT NULL DEFAULT 0,
    error        text,
    payload      jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at   timestamptz NOT NULL DEFAULT now(),
    started_at   timestamptz,
    finished_at  timestamptz
);

CREATE INDEX jobs_queue_idx ON jobs (state, created_at) WHERE state = 'queued';
CREATE INDEX jobs_document_idx ON jobs (document_id, created_at DESC);

-- ─────────────────────────────────────────────────────────────
-- Conversation log: audit + eval data, for free
-- ─────────────────────────────────────────────────────────────
CREATE TABLE conversations (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    title      text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE messages (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    conversation_id uuid REFERENCES conversations(id) ON DELETE CASCADE,
    role            text NOT NULL CHECK (role IN ('user', 'assistant')),
    content         text NOT NULL,
    route           text,          -- factual | procedural | chit_chat | out_of_scope
    latency_ms      integer,
    model           text,
    created_at      timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX messages_conversation_idx ON messages (conversation_id, created_at);

-- Citations are rendered as TEXT + source label only. `kind = 'pdf'` has no
-- url by design: students never open the source file.
CREATE TABLE citations (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    message_id   uuid NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    chunk_id     uuid REFERENCES chunks(id) ON DELETE SET NULL,
    document_id  uuid REFERENCES documents(id) ON DELETE SET NULL,
    title        text NOT NULL,
    section_path text NOT NULL DEFAULT '',
    kind         text NOT NULL,
    url          text,
    page_label   text,
    quote        text NOT NULL,
    score        real,
    ord          integer NOT NULL DEFAULT 0
);

CREATE INDEX citations_message_idx ON citations (message_id, ord);

CREATE TABLE feedback (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    message_id uuid NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    verdict    smallint NOT NULL CHECK (verdict IN (-1, 1)),
    note       text,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- ─────────────────────────────────────────────────────────────
-- App settings: content version, last sync, feature flags
-- ─────────────────────────────────────────────────────────────
CREATE TABLE app_settings (
    key        text PRIMARY KEY,
    value      jsonb NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);

INSERT INTO app_settings (key, value)
VALUES ('content_version', '"0"'), ('last_sync_at', 'null')
ON CONFLICT (key) DO NOTHING;

-- ─────────────────────────────────────────────────────────────
-- Only published chunks are ever searchable. One view, one rule.
-- ─────────────────────────────────────────────────────────────
CREATE VIEW searchable_chunks AS
SELECT
    c.id,
    c.document_id,
    c.ord,
    c.text,
    c.section_path,
    c.page_from,
    c.page_to,
    c.chunk_type,
    c.embedding,
    c.tsv,
    d.title,
    d.kind,
    d.canonical_url,
    d.category,
    d.last_verified_at
FROM chunks c
JOIN documents d ON d.id = c.document_id
WHERE d.status = 'published'
  AND (d.valid_until IS NULL OR d.valid_until > current_date);
