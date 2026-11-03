-- WP-05: content sync needs a stable identity per file on disk.
-- Uploads keep source_path = NULL (hash-addressed, no path identity).

alter table documents add column if not exists source_path text;

-- One live row per path. Archived rows are exempt so a changed file can keep history.
create unique index if not exists documents_source_path_live_uidx
    on documents (source_path)
    where source_path is not null and status <> 'archived';

create index if not exists documents_status_created_idx
    on documents (status, created_at desc);
