---
name: documents
description: Create, read, and open PDF, DOCX, PPTX, and XLSX files in Hi-Fi.
---

# Work with documents in Hi-Fi

Use `doc_create` to write a document, `doc_read` to inspect one, and `doc_open` to show a file in a user-visible Hi-Fi tab. After creating a file, call `doc_open` so the user sees it.

You can also use the CLI:

```sh
hifi doc create /path/to/brief.docx --json brief.json
cat brief.json | hifi doc create /path/to/brief.docx --json -
hifi doc read /path/to/brief.docx
hifi doc open /path/to/brief.docx
```

The file extension and `kind` must match. `doc_open` needs the Hi-Fi app to be running; create and read work without it.

## Document JSON

The `document` argument to `doc_create` has this shape:

```json
{
  "kind": "pdf | docx | pptx | xlsx",
  "title": "string",
  "blocks": [
    {"type": "heading", "level": 1, "text": "string"},
    {"type": "paragraph", "text": "string"},
    {"type": "bullet", "text": "string"},
    {"type": "numbered", "text": "string"},
    {"type": "quote", "text": "string"},
    {"type": "code", "text": "string"},
    {"type": "table", "rows": [["string", "string"]]},
    {"type": "page_break"}
  ],
  "slides": [
    {"title": "string", "bullets": ["string"], "notes": "string"}
  ],
  "sheets": [
    {"name": "string", "rows": [["string", 42, "=SUM(B1:B2)"]]}
  ]
}
```

All three collections are optional. Use `blocks` for PDF and DOCX, `slides` for PPTX, and `sheets` for XLSX. Cells are strings or numbers; a string beginning with `=` is written as a spreadsheet formula. Wrap text in `**double asterisks**` to make it bold in supported document text.

### PDF

```json
{
  "kind": "pdf",
  "title": "Quarterly summary",
  "blocks": [
    {"type": "heading", "level": 1, "text": "Quarterly summary"},
    {"type": "paragraph", "text": "Revenue grew **12%** this quarter."},
    {"type": "bullet", "text": "Renewals led growth."},
    {"type": "table", "rows": [["Quarter", "Revenue"], ["Q1", "$120,000"]]}
  ]
}
```

PDF files open read-only.

### DOCX

```json
{
  "kind": "docx",
  "title": "Launch brief",
  "blocks": [
    {"type": "heading", "level": 1, "text": "Launch brief"},
    {"type": "paragraph", "text": "The **June** release includes:"},
    {"type": "bullet", "text": "A faster export flow"},
    {"type": "numbered", "text": "Confirm the release owner"},
    {"type": "quote", "text": "Ship a small, reliable first version."},
    {"type": "code", "text": "hifi doc open launch.docx"},
    {"type": "table", "rows": [["Owner", "Area"], ["Mina", "Export"]]},
    {"type": "page_break"}
  ]
}
```

### PPTX

```json
{
  "kind": "pptx",
  "title": "Launch review",
  "slides": [
    {
      "title": "June launch",
      "bullets": ["Export is ready", "Support is staffed"],
      "notes": "Confirm the final release date."
    },
    {
      "title": "Next steps",
      "bullets": ["Review feedback"],
      "notes": ""
    }
  ]
}
```

### XLSX

```json
{
  "kind": "xlsx",
  "title": "Monthly forecast",
  "sheets": [
    {
      "name": "Revenue",
      "rows": [
        ["Month", "Revenue"],
        ["January", 120000],
        ["February", 135000],
        ["Total", "=SUM(B2:B3)"]
      ]
    }
  ]
}
```

## MCP tools

- `doc_create` takes `{ "path": "/absolute/path/report.docx", "document": { ... } }`.
- `doc_read` takes `{ "path": "/absolute/path/report.docx" }` and returns the document JSON.
- `doc_open` takes `{ "path": "/absolute/path/report.docx" }` and opens a normal tab visible to the user.

If creation fails, check that the extension matches `kind` and that the destination directory is writable. If opening fails, start Hi-Fi and retry `doc_open`.
