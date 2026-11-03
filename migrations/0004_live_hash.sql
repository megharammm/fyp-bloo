-- Dedupe must only apply to live documents. An archived version may share a hash with a
-- re-added file (sync revives content, uploads re-add deleted files).

drop index if exists documents_file_hash_uidx;

create unique index if not exists documents_file_hash_live_uidx
    on documents (file_hash)
    where status <> 'archived';
