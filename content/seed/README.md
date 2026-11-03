# Demo corpus

Everything here is re-ingested by `scripts/sync.ps1` (or the **Sync content folder** button in
`/admin`). Only new or changed files are processed — the sha256 of each file is compared
against `documents.file_hash`.

```
content/seed/
├─ manifest.json          optional metadata: title, category, valid_until
├─ admissions/
├─ fees/
├─ hostel/
├─ exams/
├─ academics/
├─ library/
└─ transport/
```

## manifest.json (optional but recommended)

If a file has no entry, Bloo derives the title from the filename and the category from the
folder name. Use the manifest to override both, and to set an expiry.

```json
{
  "files": [
    {
      "path": "fees/hostel-fee-structure-2026-27.pdf",
      "title": "Hostel Fee Structure 2026-27",
      "category": "fees",
      "valid_until": "2027-03-31",
      "last_verified_at": "2026-08-12"
    }
  ]
}
```

## Rules

- Text-based PDFs only for now. Scanned images are skipped with `status='failed'` until OCR
  lands (explicitly out of scope for v1).
- Keep individual files under 20 MB.
- PDFs and DOCX are tracked with Git LFS — run `git lfs install` once per machine, or the
  clone will contain pointer files instead of documents.
- Do not put personal data (marksheets, phone lists, medical records) in here.
