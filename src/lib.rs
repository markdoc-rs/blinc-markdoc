//! Renders a Markdoc document as Blinc UI elements.
//!
//! Input is the renderable tree of the [`markdoc`] crate (`parse` → `transform`),
//! the same tree a site generator or a PDF renderer consumes. Output is a Blinc
//! [`Div`] laid out for a given width. Besides CommonMark (headings, paragraphs,
//! emphasis, code, links, lists, pipe and list tables, block quotes, rules) it
//! draws a small set of documentation tags -- [`DOC_TAGS`]: `notice`, `image` with
//! its sizes, `imagegrid`/`gridimage`, and `tag`/`tagref` cross-references within
//! the page. A document whose config does not declare them still renders; markdoc
//! drops a tag it does not know, so the content comes through as plain blocks.
//!
//! The crate knows nothing about where images live: an [`Images`] resolver maps an
//! asset id (`{% image id="…" /%}`) or a Markdown image source to a file or SVG and
//! its pixel size. Colours and sizes come in a [`Style`], so the caller decides how
//! the document matches its theme.
//!
//! ```ignore
//! let doc = markdoc::parse(source, &markdoc::parser::ParserArgs::default())?;
//! let tree = markdoc::transform(&doc, &markdoc::schema::Config::default())?.to_render()?;
//! let body = blinc_markdoc::render(&tree, 560.0, &style, &images);
//! ```

use std::collections::HashMap;

use blinc_core::Color;
use blinc_layout::prelude::*;
use blinc_layout::styled_text::{StyledLine, StyledText, TextSpan};
pub use markdoc;
use markdoc::types::{RenderableTreeNode as Node, Scalar, Tag};

/// Colours and sizes of a rendered document.
#[derive(Clone, Debug)]
pub struct Style {
    pub text: Color,
    /// List markers, placeholders, quiet text.
    pub text_secondary: Color,
    pub link: Color,
    /// Code block and table header background.
    pub code_bg: Color,
    pub border: Color,
    /// Body text size (px); headings scale from it.
    pub body_size: f32,
    /// Vertical space between blocks (px).
    pub block_gap: f32,
    /// Notice colours by type: warning (red), caution (amber), notice (blue), info
    /// (grey) and note (light grey).
    pub warning: Color,
    pub caution: Color,
    pub notice: Color,
    pub info: Color,
    pub note: Color,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            text: Color::from_hex(0x15191B),
            text_secondary: Color::from_hex(0x50585D),
            link: Color::from_hex(0x2563EB),
            code_bg: Color::from_hex(0xF1F3F5),
            border: Color::from_hex(0xE3E6E9),
            body_size: 14.0,
            block_gap: 10.0,
            warning: Color::from_hex(0xDC2626),
            caution: Color::from_hex(0xD97706),
            notice: Color::from_hex(0x2563EB),
            info: Color::from_hex(0x6B7280),
            note: Color::from_hex(0x9CA3AF),
        }
    }
}

/// Where an image's pixels come from.
#[derive(Clone, Debug)]
pub enum ImageSource {
    /// A raster file on disk, as Blinc's image loader reads it.
    ///
    /// Blinc (rev def48dc) draws image elements in its main pass, below any
    /// `overlay_root()` ancestor's own background, so inside an overlay (a side
    /// panel, a dialog) they are hidden. Use [`ImageSource::Embedded`] there.
    File(String),
    /// SVG markup, drawn as vectors.
    Svg(String),
    /// SVG markup wrapping a raster image (`<image href="data:…">`; needs
    /// blinc_svg's `svg_raster_images` feature). Drawn like an SVG, so it shows in
    /// overlays too, but never scaled up past the image's own width.
    Embedded(String),
}

/// A resolved image: its source and natural size in pixels.
#[derive(Clone, Debug)]
pub struct Image {
    pub source: ImageSource,
    pub width: u32,
    pub height: u32,
}

/// Finds the images a document refers to.
pub trait Images {
    /// `{% image id="…" /%}` / `{% gridimage id="…" %}`: an asset id.
    fn by_id(&self, id: &str) -> Option<Image>;
    /// A Markdown image, `![alt](src)`.
    fn by_src(&self, _src: &str) -> Option<Image> {
        None
    }
}

/// No images: every image renders as its placeholder.
pub struct NoImages;

impl Images for NoImages {
    fn by_id(&self, _id: &str) -> Option<Image> {
        None
    }
}

/// `{% image size="…" %}`: how wide the image is drawn, as a share of the column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageSize {
    /// The full width (the default).
    Large,
    /// 75% of the width.
    Medium,
    /// 45% of the width.
    Small,
    /// 100 px wide: thumbnails in a table.
    Table,
    /// 18 px tall: small icons.
    Icon,
}

impl ImageSize {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "large" => Self::Large,
            "medium" => Self::Medium,
            "small" => Self::Small,
            "table" => Self::Table,
            "icon" => Self::Icon,
            _ => return None,
        })
    }

    /// Drawn size of an image `natural` (w, h) px in a column `width` px wide.
    /// Raster images are not scaled up past their own width (they would blur).
    pub fn fit(self, natural: (u32, u32), width: f32, vector: bool) -> (f32, f32) {
        let (nw, nh) = (natural.0.max(1) as f32, natural.1.max(1) as f32);
        let w = match self {
            Self::Large => width,
            Self::Medium => width * 0.75,
            Self::Small => width * 0.45,
            Self::Table => 100.0_f32.min(width),
            Self::Icon => 18.0 * nw / nh,
        };
        let w = if vector { w } else { w.min(nw) };
        (w, w * nh / nw)
    }
}

/// The tags this renderer draws (beyond the HTML-like nodes of CommonMark): a
/// document using others still renders, their content shown as plain blocks.
pub const DOC_TAGS: &[&str] = &[
    "notice",
    "image",
    "imagegrid",
    "gridimage",
    "tag",
    "tagref",
    "pagebreak",
];

/// Renders `tree` (a transformed Markdoc document) `width` px wide.
pub fn render(tree: &Node, width: f32, style: &Style, images: &dyn Images) -> Div {
    let anchors = anchors(tree);
    let cx = Cx {
        style,
        images,
        anchors: &anchors,
    };
    cx.flow(children(tree), width)
}

/// Whether this renderer draws a node of the transformed tree: the HTML-like nodes
/// of CommonMark and [`DOC_TAGS`]. Others render as their content.
pub fn is_supported(name: &str) -> bool {
    DOC_TAGS.contains(&name)
        || matches!(
            name,
            // `article` is the document root markdoc transforms to.
            "article"
                | "div"
                | "p"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "ul"
                | "ol"
                | "li"
                | "table"
                | "thead"
                | "tbody"
                | "tfoot"
                | "tr"
                | "th"
                | "td"
                | "pre"
                | "blockquote"
                | "hr"
                | "img"
                | "strong"
                | "b"
                | "em"
                | "i"
                | "code"
                | "a"
                | "s"
                | "del"
                | "span"
                | "softbreak"
                | "br"
                | "hardbreak"
        )
}

/// Every node name used in `tree`, for checking documents against [`is_supported`].
pub fn tag_names(tree: &Node) -> Vec<String> {
    fn walk(n: &Node, out: &mut Vec<String>) {
        if let Node::Tag(t) = n {
            if !out.contains(&t.name) {
                out.push(t.name.clone());
            }
            for c in &t.children {
                walk(c, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(tree, &mut out);
    out
}

struct Cx<'a> {
    style: &'a Style,
    images: &'a dyn Images,
    /// `{% tag %}` name → the text of the heading it sits on.
    anchors: &'a HashMap<String, String>,
}

fn children(n: &Node) -> &[Node] {
    match n {
        Node::Tag(t) => &t.children,
        Node::Scalar(_) => &[],
    }
}

fn str_attr<'t>(t: &'t Tag, name: &str) -> Option<&'t str> {
    match t.attributes.get(name)? {
        Scalar::String(s) => Some(s),
        _ => None,
    }
}

/// Phrasing content: becomes part of a line of text.
fn is_inline(n: &Node) -> bool {
    match n {
        Node::Scalar(_) => true,
        Node::Tag(t) => matches!(
            t.name.as_str(),
            "strong"
                | "b"
                | "em"
                | "i"
                | "code"
                | "a"
                | "s"
                | "del"
                | "span"
                | "softbreak"
                | "br"
                | "hardbreak"
                | "tag"
                | "tagref"
        ),
    }
}

fn plain_text(n: &Node, out: &mut String) {
    match n {
        Node::Scalar(Scalar::String(s)) => out.push_str(s),
        Node::Scalar(Scalar::Number(v)) => out.push_str(&v.to_string()),
        Node::Scalar(_) => {}
        Node::Tag(t) => match t.name.as_str() {
            "tag" => {}
            "softbreak" => out.push(' '),
            _ => t.children.iter().for_each(|c| plain_text(c, out)),
        },
    }
}

/// Headings carrying a `{% tag "name" /%}`, by name, for `{% tagref %}` link text.
fn anchors(tree: &Node) -> HashMap<String, String> {
    fn walk(n: &Node, out: &mut HashMap<String, String>) {
        let Node::Tag(t) = n else { return };
        if matches!(t.name.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
            let names = t.children.iter().filter_map(|c| match c {
                Node::Tag(c) if c.name == "tag" => str_attr(c, "primary").or(str_attr(c, "id")),
                _ => None,
            });
            let mut text = String::new();
            plain_text(n, &mut text);
            for name in names {
                out.insert(name.to_owned(), text.trim().to_owned());
            }
        }
        t.children.iter().for_each(|c| walk(c, out));
    }
    let mut out = HashMap::new();
    walk(tree, &mut out);
    out
}

/// Text with styled ranges, built from inline nodes.
#[derive(Default)]
struct Inline {
    lines: Vec<(String, Vec<TextSpan>)>,
    text: String,
    spans: Vec<TextSpan>,
}

#[derive(Clone, Copy, Default)]
struct Marks<'u> {
    bold: bool,
    italic: bool,
    code: bool,
    strike: bool,
    link: Option<&'u str>,
    /// A cross-reference: link-coloured, but nothing to open.
    xref: bool,
}

impl Inline {
    fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.text.trim().is_empty()
    }

    fn push(&mut self, s: &str, m: Marks<'_>, style: &Style) {
        if s.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(s);
        let link = m.link.is_some() || m.xref;
        let color = if link { style.link } else { style.text };
        let mut span = TextSpan::new(start, self.text.len(), color, m.bold)
            .with_italic(m.italic)
            .with_strikethrough(m.strike)
            .with_underline(m.link.is_some())
            .with_code(m.code);
        if let Some(url) = m.link {
            span = span.with_link(url);
        }
        self.spans.push(span);
    }

    fn line_break(&mut self) {
        let text = std::mem::take(&mut self.text);
        let spans = std::mem::take(&mut self.spans);
        self.lines.push((text, spans));
    }

    fn take(&mut self) -> Inline {
        std::mem::take(self)
    }
}

impl Cx<'_> {
    /// Block content: runs of inline nodes become paragraphs, everything else a block.
    fn flow(&self, nodes: &[Node], width: f32) -> Div {
        self.flow_gap(nodes, width, self.style.block_gap)
    }

    fn flow_gap(&self, nodes: &[Node], width: f32, gap: f32) -> Div {
        let mut col = div().w(width).flex_col().gap_px(gap);
        let mut run = Inline::default();
        for n in nodes {
            if is_inline(n) {
                self.inline(n, Marks::default(), &mut run);
                continue;
            }
            if !run.is_empty() {
                col = col.child(self.paragraph(run.take(), self.style.body_size, false));
            }
            run = Inline::default();
            for block in self.block(n, width) {
                col = col.child_box(block);
            }
        }
        if !run.is_empty() {
            col = col.child(self.paragraph(run, self.style.body_size, false));
        }
        col
    }

    fn inline<'n>(&self, n: &'n Node, m: Marks<'n>, out: &mut Inline) {
        let t = match n {
            Node::Scalar(Scalar::String(s)) => return out.push(s, m, self.style),
            Node::Scalar(Scalar::Number(v)) => return out.push(&v.to_string(), m, self.style),
            Node::Scalar(_) => return,
            Node::Tag(t) => t,
        };
        let mut m = m;
        match t.name.as_str() {
            "strong" | "b" => m.bold = true,
            "em" | "i" => m.italic = true,
            "s" | "del" => m.strike = true,
            "code" => m.code = true,
            "a" => m.link = str_attr(t, "href"),
            "softbreak" => return out.push(" ", m, self.style),
            "br" | "hardbreak" => return out.line_break(),
            "tag" => return,
            "tagref" => {
                let name = str_attr(t, "primary").or(str_attr(t, "id")).unwrap_or("");
                let label = str_attr(t, "text")
                    .map(str::to_owned)
                    .or_else(|| self.anchors.get(name).cloned())
                    .unwrap_or_else(|| name.replace('_', " "));
                m.xref = true;
                return out.push(&label, m, self.style);
            }
            _ => {}
        }
        for c in &t.children {
            self.inline(c, m, out);
        }
    }

    fn paragraph(&self, mut run: Inline, size: f32, bold: bool) -> RichText {
        // Leading/trailing blanks of a line come from source line ends.
        run.line_break();
        let lines = run
            .lines
            .into_iter()
            .map(|(text, spans)| {
                let lead = text.len() - text.trim_start().len();
                let trimmed = text.trim().to_owned();
                let spans = spans
                    .into_iter()
                    .filter_map(|mut s| {
                        s.start = s.start.saturating_sub(lead).min(trimmed.len());
                        s.end = s.end.saturating_sub(lead).min(trimmed.len());
                        (s.start < s.end).then_some(s)
                    })
                    .collect();
                StyledLine::new(trimmed, spans)
            })
            .collect();
        let mut rt = rich_text_styled(StyledText::from_lines(lines))
            .size(size)
            .default_color(self.style.text)
            .line_height(1.45);
        if bold {
            rt = rt.weight(FontWeight::Bold);
        }
        rt
    }

    fn block(&self, n: &Node, width: f32) -> Vec<Box<dyn ElementBuilder>> {
        let Node::Tag(t) = n else { return Vec::new() };
        let s = self.style;
        let one = |e: Box<dyn ElementBuilder>| vec![e];
        match t.name.as_str() {
            "p" => self.paragraph_with_images(t, width),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = t.name.as_bytes()[1] - b'0';
                let size = s.body_size
                    * match level {
                        1 => 1.6,
                        2 => 1.35,
                        3 => 1.15,
                        _ => 1.0,
                    };
                let mut run = Inline::default();
                for c in &t.children {
                    self.inline(c, Marks::default(), &mut run);
                }
                let top = if level <= 2 {
                    s.block_gap
                } else {
                    s.block_gap / 2.0
                };
                one(Box::new(
                    div()
                        .w(width)
                        .pt(top / 4.0)
                        .child(self.paragraph(run, size, true)),
                ))
            }
            "ul" | "ol" => one(Box::new(self.list(t, width))),
            "table" => one(Box::new(self.table(t, width))),
            "pre" => {
                let mut code = String::new();
                plain_text(n, &mut code);
                let lines = code
                    .trim_end_matches('\n')
                    .lines()
                    .map(|l| StyledLine::plain(l, s.text))
                    .collect();
                one(Box::new(
                    div().w(width).p_px(10.0).rounded(6.0).bg(s.code_bg).child(
                        rich_text_styled(StyledText::from_lines(lines))
                            .size(s.body_size - 1.0)
                            .default_color(s.text)
                            .monospace()
                            .line_height(1.4),
                    ),
                ))
            }
            "blockquote" => one(Box::new(
                div()
                    .w(width)
                    .pl(3.0)
                    .border_left(3.0, s.border)
                    .child(self.flow(&t.children, width - 15.0)),
            )),
            "hr" => one(Box::new(div().w(width).h(1.0).bg(s.border))),
            "notice" => one(Box::new(self.notice(t, width))),
            "image" | "img" => self.image(t, width).into_iter().collect(),
            "imagegrid" => one(Box::new(self.image_grid(t, width))),
            // A PDF page break; nothing on screen.
            "pagebreak" => Vec::new(),
            // Unknown blocks (and the document root): their content.
            _ => one(Box::new(self.flow(&t.children, width))),
        }
    }

    /// A paragraph, split where it holds images (`A result: {% image … /%}`).
    fn paragraph_with_images(&self, t: &Tag, width: f32) -> Vec<Box<dyn ElementBuilder>> {
        let mut out: Vec<Box<dyn ElementBuilder>> = Vec::new();
        let mut run = Inline::default();
        for c in &t.children {
            match c {
                Node::Tag(img) if matches!(img.name.as_str(), "image" | "img") => {
                    if !run.is_empty() {
                        out.push(Box::new(self.paragraph(
                            run.take(),
                            self.style.body_size,
                            false,
                        )));
                    }
                    run = Inline::default();
                    out.extend(self.image(img, width));
                }
                _ => self.inline(c, Marks::default(), &mut run),
            }
        }
        if !run.is_empty() {
            out.push(Box::new(self.paragraph(run, self.style.body_size, false)));
        }
        out
    }

    fn list(&self, t: &Tag, width: f32) -> Div {
        let s = self.style;
        let ordered = t.name == "ol";
        let start = match t.attributes.get("start") {
            Some(Scalar::Number(n)) => *n as i64,
            _ => 1,
        };
        let marker_w = if ordered { 26.0 } else { 16.0 };
        let mut col = div().w(width).flex_col().gap_px(s.block_gap * 0.6);
        let items = t
            .children
            .iter()
            .filter(|c| matches!(c, Node::Tag(li) if li.name == "li"));
        for (i, item) in items.enumerate() {
            let marker = if ordered {
                format!("{}.", start + i as i64)
            } else {
                "•".to_owned()
            };
            col = col.child(
                div()
                    .w(width)
                    .flex_row()
                    .items_start()
                    .child(
                        div().w(marker_w).flex_shrink_0().child(
                            text(marker)
                                .size(s.body_size)
                                .color(s.text_secondary)
                                .line_height(1.45),
                        ),
                    )
                    .child(self.flow_gap(children(item), width - marker_w, s.block_gap * 0.6)),
            );
        }
        col
    }

    fn table(&self, t: &Tag, width: f32) -> Div {
        let s = self.style;
        // Rows: `tr`s anywhere below the table; a `thead` holding cells directly (a
        // pipe table) is itself the header row.
        fn rows<'t>(t: &'t Tag, head: bool, out: &mut Vec<(bool, &'t Tag)>) {
            for c in &t.children {
                let Node::Tag(c) = c else { continue };
                match c.name.as_str() {
                    "tr" => out.push((head, c)),
                    "thead" if c.children.iter().any(is_cell) => out.push((true, c)),
                    "thead" => rows(c, true, out),
                    "tbody" | "tfoot" => rows(c, false, out),
                    _ => {}
                }
            }
        }
        fn is_cell(n: &Node) -> bool {
            matches!(n, Node::Tag(c) if c.name == "td" || c.name == "th")
        }
        let mut all = Vec::new();
        rows(t, false, &mut all);
        let cols = all
            .iter()
            .map(|(_, r)| r.children.iter().filter(|c| is_cell(c)).count())
            .max()
            .unwrap_or(0);
        if cols == 0 {
            return div();
        }
        let weights: Vec<f32> = str_attr(t, "column_weights")
            .map(|w| {
                w.split([' ', ','])
                    .filter_map(|x| x.trim().parse::<f32>().ok())
                    .filter(|x| *x > 0.0)
                    .collect()
            })
            .filter(|w: &Vec<f32>| w.len() == cols)
            .unwrap_or_else(|| vec![1.0; cols]);
        let total: f32 = weights.iter().sum();
        let pad = 8.0;
        let mut table = div()
            .w(width)
            .flex_col()
            .border(1.0, s.border)
            .rounded(6.0)
            .overflow_clip();
        for (i, (head, row)) in all.iter().enumerate() {
            let mut r = div().w(width).flex_row();
            if *head {
                r = r.bg(s.code_bg);
            }
            if i > 0 {
                r = r.border_top(1.0, s.border);
            }
            let cells = row.children.iter().filter(|c| is_cell(c));
            for (j, cell) in cells.enumerate() {
                let w = (width - 2.0) * weights[j] / total;
                let content = if *head {
                    let mut run = Inline::default();
                    let bold = Marks {
                        bold: true,
                        ..Marks::default()
                    };
                    for c in children(cell) {
                        self.inline(c, bold, &mut run);
                    }
                    div().child(self.paragraph(run, s.body_size - 1.0, false))
                } else {
                    self.flow_gap(children(cell), w - 2.0 * pad, s.block_gap * 0.6)
                };
                let mut c = div().w(w).p_px(pad).flex_shrink_0().child(content);
                if j > 0 {
                    c = c.border_left(1.0, s.border);
                }
                r = r.child(c);
            }
            table = table.child(r);
        }
        table
    }

    fn notice(&self, t: &Tag, width: f32) -> Div {
        let s = self.style;
        let kind = str_attr(t, "type").unwrap_or("note");
        let (color, icon, label) = match kind {
            "warning" => (s.warning, blinc_icons::icons::TRIANGLE_ALERT, "WARNING"),
            "caution" => (s.caution, blinc_icons::icons::TRIANGLE_ALERT, "CAUTION"),
            "notice" => (s.notice, blinc_icons::icons::INFO, "NOTICE"),
            "info" => (s.info, blinc_icons::icons::INFO, "INFO"),
            _ => (s.note, blinc_icons::icons::NOTEBOOK_PEN, "NOTE"),
        };
        let pad = 12.0;
        let svg_icon = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{icon}</svg>"#
        );
        div()
            .w(width)
            .flex_col()
            .gap_px(s.block_gap * 0.6)
            .p_px(pad)
            .rounded(6.0)
            .bg(color.with_alpha(0.08))
            .border_left(4.0, color)
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .gap_px(6.0)
                    .child(svg(&svg_icon).size(16.0, 16.0).color(color))
                    .child(
                        text(label)
                            .size(s.body_size - 2.0)
                            .weight(FontWeight::Bold)
                            .color(color),
                    ),
            )
            .child(self.flow(&t.children, width - 2.0 * pad - 4.0))
    }

    fn resolve(&self, t: &Tag) -> Option<Image> {
        match t.name.as_str() {
            "img" => self.images.by_src(str_attr(t, "src")?),
            _ => self.images.by_id(str_attr(t, "id")?),
        }
    }

    fn image_element(&self, image: Image, size: ImageSize, width: f32) -> Box<dyn ElementBuilder> {
        let vector = matches!(image.source, ImageSource::Svg(_));
        let (w, h) = size.fit((image.width, image.height), width, vector);
        match image.source {
            ImageSource::File(path) => Box::new(img(path).size(w, h).contain().no_fade()),
            ImageSource::Svg(src) | ImageSource::Embedded(src) => Box::new(svg(&src).size(w, h)),
        }
    }

    fn placeholder(&self, what: &str, width: f32) -> Box<dyn ElementBuilder> {
        Box::new(
            div()
                .w(width.min(240.0))
                .h(60.0)
                .rounded(6.0)
                .border(1.0, self.style.border)
                .items_center()
                .justify_center()
                .child(
                    text(format!("Missing image {what}"))
                        .size(self.style.body_size - 2.0)
                        .color(self.style.text_secondary),
                ),
        )
    }

    fn image(&self, t: &Tag, width: f32) -> Option<Box<dyn ElementBuilder>> {
        let size = str_attr(t, "size")
            .and_then(ImageSize::parse)
            .unwrap_or(ImageSize::Large);
        Some(match self.resolve(t) {
            Some(image) => self.image_element(image, size, width),
            None => {
                let what = str_attr(t, "id").or(str_attr(t, "src")).unwrap_or("");
                self.placeholder(what, width)
            }
        })
    }

    fn image_grid(&self, t: &Tag, width: f32) -> Div {
        let s = self.style;
        let cells: Vec<&Tag> = t
            .children
            .iter()
            .filter_map(|c| match c {
                Node::Tag(g) if g.name == "gridimage" => Some(&**g),
                _ => None,
            })
            .collect();
        let gap = 12.0;
        let n = cells.len().max(1) as f32;
        let cell_w = (width - gap * (n - 1.0)) / n;
        let mut row = div().w(width).flex_row().items_start().gap_px(gap);
        for g in cells {
            let mut cell = div().w(cell_w).flex_col().gap_px(s.block_gap * 0.5);
            cell = cell.child_box(match self.resolve(g) {
                Some(image) => self.image_element(image, ImageSize::Large, cell_w),
                None => self.placeholder(str_attr(g, "id").unwrap_or(""), cell_w),
            });
            // A paragraph that is all bold is the caption; the rest is body text.
            for p in &g.children {
                let Node::Tag(pt) = p else { continue };
                let caption = pt.children.len() == 1
                    && matches!(&pt.children[0], Node::Tag(b) if b.name == "strong");
                let mut run = Inline::default();
                for c in &pt.children {
                    self.inline(c, Marks::default(), &mut run);
                }
                let size = if caption {
                    s.body_size
                } else {
                    s.body_size - 1.0
                };
                cell = cell.child(self.paragraph(run, size, false));
            }
            row = row.child(cell);
        }
        row
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The documentation tags, declared as a document's own config has to declare
    /// them: markdoc drops a tag it does not know, so without this the tree holds
    /// the content and none of the tags.
    fn config() -> markdoc::schema::Config {
        use markdoc::schema::{Schema, SchemaAttribute, Value};
        let tag = |name: &str, self_closing: bool, attrs: &[&str]| {
            Some(Schema {
                render: Some(Value::from(name)),
                attributes: Some(
                    attrs
                        .iter()
                        .map(|a| ((*a).to_string(), SchemaAttribute::default()))
                        .collect(),
                ),
                self_closing,
                ..Default::default()
            })
        };
        markdoc::schema::Config {
            tags: [
                ("notice", tag("notice", false, &["type"])),
                ("image", tag("image", true, &["id", "src", "size", "alt"])),
                ("imagegrid", tag("imagegrid", false, &[])),
                (
                    "gridimage",
                    tag("gridimage", true, &["id", "alt", "headline", "bodytext"]),
                ),
                ("tag", tag("tag", true, &["primary", "id", "text"])),
                ("tagref", tag("tagref", true, &["primary", "id", "text"])),
                ("pagebreak", tag("pagebreak", true, &[])),
            ]
            .into_iter()
            .map(|(name, schema)| (name.to_string(), schema))
            .collect(),
            ..Default::default()
        }
    }

    fn tree(src: &str) -> Node {
        let doc = markdoc::parse(src, &markdoc::parser::ParserArgs::default()).unwrap();
        let value = markdoc::transform(&doc, &config()).unwrap();
        value.to_render().unwrap()
    }

    #[test]
    fn the_documentation_tags_are_supported() {
        let t = tree(
            "{% notice type=\"info\" %}\nHi.\n{% /notice %}\n\n{% image id=\"a\" size=\"small\" /%}\n\n{% imagegrid %}\n{% gridimage id=\"b\" /%}\n{% /imagegrid %}\n\n* a\n\n| x |\n| --- |\n| 1 |\n",
        );
        let names = tag_names(&t);
        for n in ["notice", "image", "imagegrid", "gridimage", "ul", "table"] {
            assert!(names.iter().any(|x| x == n), "{n} in {names:?}");
        }
        assert!(names.iter().all(|n| is_supported(n)), "{names:?}");
        assert!(!is_supported("spare_part_finder"));
    }

    #[test]
    fn tagref_uses_the_heading_text() {
        let t = tree("## Fine yaw {% tag \"fine_yaw\" /%}\n\nSee {% tagref \"fine_yaw\" /%}.");
        assert_eq!(
            anchors(&t).get("fine_yaw").map(String::as_str),
            Some("Fine yaw")
        );
    }

    #[test]
    fn image_sizes_follow_the_documented_widths() {
        let (w, h) = ImageSize::Medium.fit((1000, 500), 400.0, false);
        assert_eq!((w, h), (300.0, 150.0));
        // Raster images are not scaled up; vectors are.
        assert_eq!(
            ImageSize::Large.fit((200, 100), 400.0, false),
            (200.0, 100.0)
        );
        assert_eq!(
            ImageSize::Large.fit((200, 100), 400.0, true),
            (400.0, 200.0)
        );
        assert_eq!(
            ImageSize::Table.fit((400, 400), 400.0, false),
            (100.0, 100.0)
        );
        assert_eq!(ImageSize::parse("tiny"), None);
    }
}
