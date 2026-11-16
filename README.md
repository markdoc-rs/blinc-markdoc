# blinc-markdoc

Renders a Markdoc document as [Blinc](https://github.com/project-blinc/Blinc)
UI elements.

Input is the renderable tree of the [`markdoc`](https://github.com/markdoc-rs/markdoc)
crate — the same tree a site generator or a PDF renderer consumes — so a
document reads the same on screen as it does in print, from one source.

```rust
let doc = markdoc::parse(source, &markdoc::parser::ParserArgs::default())?;
let tree = markdoc::transform(&doc, &config)?.to_render()?;
let body = blinc_markdoc::render(&tree, 560.0, &style, &images);
```

## What it draws

All of CommonMark that a document uses: headings, paragraphs, emphasis, code,
links, lists, pipe and list tables, block quotes and rules.

Beyond that, a small set of documentation tags — `blinc_markdoc::DOC_TAGS`:

| tag | |
|---|---|
| `{% notice type="…" %}` | a coloured callout: warning, caution, notice, info, note |
| `{% image id="…" size="…" /%}` | an image at one of the documented widths |
| `{% imagegrid %}` / `{% gridimage %}` | images side by side, each with a headline and body |
| `{% tag "id" /%}` / `{% tagref "id" /%}` | a cross-reference within the page, labelled with the heading it points at |
| `{% pagebreak /%}` | nothing on screen; a page break in print |

These are not markdoc built-ins. A document's own `Config` declares them, and
markdoc drops a tag it does not know — so a document that declares none of
them still renders, with the content coming through as plain blocks.
`blinc_markdoc::is_supported` says whether a node is one this renderer draws.

## What it does not decide

**Where images live.** An `Images` resolver maps an asset id or a Markdown
image source to a file or an SVG and its pixel size. `NoImages` draws none.

**How it looks.** Colours, sizes and gaps come in a `Style`, so the document
matches the theme around it rather than carrying one of its own.

## License

MIT
