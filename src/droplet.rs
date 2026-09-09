use anyhow::Error;
use chrono::NaiveDate;
use fancy_regex::Regex;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use tera::Context;

use std::fmt::Debug;
use std::fs::File;
use std::path::Path;

use crate::errors;

const CONTENT_BLOCKS_REGEX_STR: &str = r"(?m)^\s*```\w*\n(?P<multiline_pre>[\s\S]*?)\n\s*```\s*$|^(?!\s*$).+(?:\r?\n(?!\s*$).+)*\r?\n?\s*$";

const EM_REGEX_STR: &str = r"_(?P<em_text>.*?)_";
const STRONG_REGEX_STR: &str = r"\*\*(?P<strong_text>.*?)\*\*";
const A_REGEX_STR: &str = r"\[(?P<a_text>.*?)\]\((?P<a_href>.*?)\)";
const CODE_INLINE_REGEX_STR: &str = r"(?<!\\)\`{1,}(?P<inline_pre>.+?)(?<!`)(?<!\\)\`{1,}(?!`)";
const CODE_MULTILINE_REGEX_STR: &str =
    r"(?m)^\s*\`\`\`(\w*)\n(?P<multiline_pre>[\s\S]*?)\n\`\`\`\s*$";
const ESCAPED_SINGLE_BACKTICK_REGEX_STR: &str = r"\\\`";
const LT_REGEX_STR: &str = r"\<";
const GT_REGEX_STR: &str = r"\>";
const AMP_REGEX_STR: &str = r"\&";

lazy_static! {
    // Unwrap these with certainty since the expressions themselves are constants and should compile just fine
    static ref CONTENT_BLOCKS_REGEX: Regex = Regex::new(CONTENT_BLOCKS_REGEX_STR).unwrap();
    static ref EM_REGEX: Regex = Regex::new(EM_REGEX_STR).unwrap();
    static ref STRONG_REGEX: Regex = Regex::new(STRONG_REGEX_STR).unwrap();
    static ref A_REGEX: Regex = Regex::new(A_REGEX_STR).unwrap();
    static ref CODE_INLINE_REGEX: Regex = Regex::new(CODE_INLINE_REGEX_STR).unwrap();
    static ref CODE_MULTILINE_REGEX: Regex = Regex::new(CODE_MULTILINE_REGEX_STR).unwrap();
    static ref ESCAPED_SINGLE_BACKTICK_REGEX: Regex = Regex::new(ESCAPED_SINGLE_BACKTICK_REGEX_STR).unwrap();
    static ref LT_REGEX: Regex = Regex::new(LT_REGEX_STR).unwrap();
    static ref GT_REGEX: Regex = Regex::new(GT_REGEX_STR).unwrap();
    static ref AMP_REGEX: Regex = Regex::new(AMP_REGEX_STR).unwrap();
}

#[derive(Debug, Deserialize, Serialize)]
/// Metadata for a Droplet
pub(crate) struct DropletMeta {
    /// An array of tags/keywords for this page
    tags: Option<Vec<String>>,
    /// The author of the post
    pub author: String,
    /// The date of the post
    pub date: Option<NaiveDate>,
    /// A private member of the meta struct, where the derived path of the page is set
    path: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
/// Image data for a Droplet
pub(crate) struct DropletImage {
    /// The path to the image relative to the directory in which the droplet .yaml exists
    src: Box<Path>,
    /// Alt text for accessibility
    alt: Option<String>,
    /// Copyright information
    copyright: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
/// A Droplet is a single post or entry.
pub(crate) struct Droplet {
    /// Metadata for this post
    meta: DropletMeta,
    /// The title of the post
    pub title: String,
    /// Image content for the post
    image: Option<DropletImage>,
    /// Text content for the post
    content: Option<String>,
}

impl DropletMeta {
    fn set_path(&mut self, path: String) {
        self.path = Some(path);
    }
}

impl Droplet {
    pub fn from_file<P>(path: P) -> Result<Droplet, Error>
    where
        P: AsRef<Path> + Debug + Send + Sync + Copy,
    {
        let droplet_file = File::open(path).map_err(|e| {
            let err_path = path.as_ref().to_owned();
            errors::Errors::InvalidDropletPath {
                source: e.into(),
                path: err_path,
            }
        })?;
        Ok(serde_yaml::from_reader(droplet_file).map_err(|e| {
            let err_path = path.as_ref().to_owned();
            errors::Errors::InvalidDropletFormat {
                source: e.into(),
                path: err_path,
            }
        })?)
    }

    fn image_to_html(&self) -> Option<String> {
        if let Some(droplet_image) = &self.image {
            let mut attr_vec: Vec<String> = Vec::new();
            attr_vec.push(format!("src=\"{}\"", droplet_image.src.as_ref().display()));
            droplet_image.alt.as_ref().map(|alt| {
                let cleaned = alt
                    .trim()
                    .split('\n')
                    .fold("".to_string(), |mut acc, alt_line| {
                        acc.push_str(alt_line);
                        acc
                    });

                attr_vec.push(format!("alt=\"{}\"", cleaned));
            });

            Some(format!(
                "<img class=\"spindrift-img\" {}/>\n",
                attr_vec.join(" ")
            ))
        } else {
            None
        }
    }

    fn content_to_html(&self) -> Option<String> {
        // A spindrift post is a multiline string in yaml, with basic markdown
        // support.
        //
        // Imagine this is a big blob of stuff that we want to parse vertically,
        // we'd see the following segments:
        //
        // 1. plain text
        // 2. plain text with inline markdown (does not cross line boundaries)
        // 3. markdown codeblocks - these cross line boundaries
        //
        // Since there could be new and empty lines in a codeblock, we first
        // need to clump our countent by "unsplittable veritcal sections".
        //
        // The `CONTENT_BLOCKS_REGEX` expression lazily finds markdown-style
        // codeblocks before greedily finding all other multiline blobs of text,
        // separated by a full line of whitespace.
        //
        // This **only** works when we can parse our yaml's `content` block to
        // multiline text, rather than singleline text, due to the way that
        // `\n` characters get dropped in the parsed string unless the subsequent
        // line begins with whitespace (or is itself a newline).
        self.content.as_ref().map(|content| {
            CONTENT_BLOCKS_REGEX
                .find_iter(content.trim())
                .map(|block| block.unwrap().as_str().trim())
                .map(|block| {
                    if !block.starts_with("```") {
                        block.replace("\n", " ").to_owned().to_string()
                    } else {
                        block.to_string()
                    }
                })
                .map(|v| {
                    // Do this first, otherwise the rest of the escaped entites get double escaped
                    let mut builder = AMP_REGEX.replace_all(&v, "&amp;").to_owned().to_string();
                    builder = GT_REGEX
                        .replace_all(&builder, "&gt;")
                        .to_owned()
                        .to_string();
                    builder = LT_REGEX
                        .replace_all(&builder, "&lt;")
                        .to_owned()
                        .to_string();
                    builder = ESCAPED_SINGLE_BACKTICK_REGEX
                        .replace_all(&builder, "&grave;")
                        .to_owned()
                        .to_string();
                    builder = EM_REGEX
                        .replace_all(&builder, "<em>$em_text</em>")
                        .to_owned()
                        .to_string();
                    builder = STRONG_REGEX
                        .replace_all(&builder, "<strong>$strong_text</strong>")
                        .to_owned()
                        .to_string();
                    builder = A_REGEX
                        .replace_all(&builder, "<a href=\"$a_href\">$a_text</a>")
                        .to_owned()
                        .to_string();
                    builder = CODE_MULTILINE_REGEX
                        .replace_all(
                            &builder,
                            "<pre class=\"droplet-codeblock\"><code>$multiline_pre</code></pre>",
                        )
                        .to_owned()
                        .to_string();
                    CODE_INLINE_REGEX
                        .replace_all(&builder, "<code>$inline_pre</code>")
                        .to_owned()
                        .to_string()
                })
                .map(|v| format!("<p class=\"droplet-text\">{}</p>", v))
                .fold("".to_string(), |mut acc, paragraph| {
                    acc.push_str(&format!("{}\n", paragraph));
                    acc
                })
        })
    }

    pub fn file_name(&self) -> String {
        format!(
            "{}{}",
            self.title
                .split_whitespace()
                // drop non-alphanumeric chars (maybe there's a better way to do this, idk)
                .map(|t| t
                    .chars()
                    .map(|c| match c {
                        'A'..='Z' => {
                            c.to_lowercase().to_string()
                        }
                        'a'..='z' | '0'..='9' => {
                            c.to_string()
                        }
                        _ => {
                            "".to_string()
                        }
                    })
                    .collect())
                .collect::<Vec<String>>()
                .join("-"),
            ".html".to_string()
        )
    }

    pub fn set_file_name(&mut self) {
        self.meta.set_path(self.file_name());
    }

    pub fn as_context(&self) -> Context {
        let mut context = Context::new();
        if let Some(html_content) = self.content_to_html() {
            context.insert("content", &html_content);
        }
        context.insert("meta", &self.meta);
        context.insert("title", &self.title);
        context
    }
}
