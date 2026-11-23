//! Thai edition: the engine's and the system's reasons, in the user's language. The engine words
//! its errors in English (PdfCraft's own strings, left as they are so upstream changes merge
//! cleanly); the app says them through [`detail`], which looks each one up in the catalog under
//! the `error` context. A reason with a part that varies (a page number, a file name, another
//! reason) is matched against [`TEMPLATES`], its parts put into the translated sentence. Anything
//! with no translation is shown as it is.

use super::{Lang, tr_ctx};

/// The catalog context of error texts, so a reason never takes the translation of a label that
/// happens to read the same.
const CONTEXT: &str = "error";

/// Reasons with parts that vary, `{0}` and `{1}`, in the engine's wording. Each is a catalog row
/// under [`CONTEXT`]; its parts are put back into the translation, `{0}` itself translated.
pub(crate) const TEMPLATES: &[&str] = &[
    "this document can't be edited: {0}",
    "the document's security settings don't allow {0}; open it with the owner password to make this change",
    "the edited document could not be written: {0}",
    "the edited document could not be reopened: {0}",
    "couldn't use {0}",
    "{0} isn't possible in a signed document: it would rewrite the file and invalidate the signatures",
    "page {0} does not exist",
    "the Thai font can't show \"{0}\"",
    "the standard fonts can't draw \"{0}\" ({1}); only Western European characters can be added as text for now",
];

/// Windows' error codes in an OS message ("… (os error 32)"), and the reason each is said as.
pub(crate) const OS_ERRORS: &[(u32, &str)] = &[
    (2, "The file was not found"),
    (3, "The folder was not found"),
    (5, "Access to the file was denied"),
    (32, "The file is open in another program"),
    (33, "Part of the file is locked by another program"),
    (112, "There is not enough space on the disk"),
    (183, "A file with that name already exists"),
    (206, "The file name or path is too long"),
    (1224, "The file is open in another program"),
];

/// The most a reason is looked through for nested reasons.
const MAX_DEPTH: usize = 3;

/// `detail` (an engine or system reason) in `lang`, or as it is when it has no translation. Only
/// Thai, the edition's language, is translated: the other languages keep PdfCraft's behaviour of
/// showing the engine's own words.
pub fn detail(lang: Lang, detail: &str) -> String {
    if lang.code() != "th" {
        return detail.to_owned();
    }
    translate(lang, detail.trim(), 0)
}

fn translate(lang: Lang, s: &str, depth: usize) -> String {
    let exact = tr_ctx(lang, CONTEXT, s);
    if exact != s {
        return exact.to_owned();
    }
    if let Some(reason) = os_error(s).and_then(|code| OS_ERRORS.iter().find(|(c, _)| *c == code)).map(|(_, r)| *r) {
        let t = tr_ctx(lang, CONTEXT, reason);
        if t != reason {
            return t.to_owned();
        }
    }
    if depth < MAX_DEPTH {
        for template in TEMPLATES {
            let Some(parts) = fill(template, s) else { continue };
            let t = tr_ctx(lang, CONTEXT, template);
            if t == *template {
                continue;
            }
            let mut out = t.to_owned();
            for (i, part) in parts.iter().enumerate() {
                // A nested reason is translated too; a name or a number stays as it is.
                let part = if i == 0 { translate(lang, part, depth + 1) } else { (*part).to_owned() };
                out = out.replace(&format!("{{{i}}}"), &part);
            }
            return out;
        }
    }
    s.to_owned()
}

/// The code in a message ending "(os error N)".
fn os_error(s: &str) -> Option<u32> {
    let start = s.rfind("(os error ")?;
    s.get(start + "(os error ".len()..)?.strip_suffix(')')?.trim().parse().ok()
}

/// The parts of `s` that `{0}` and `{1}` stand for in `template`, when `s` reads as `template`.
fn fill<'a>(template: &str, s: &'a str) -> Option<Vec<&'a str>> {
    // The template's literal pieces, between its placeholders.
    let mut pieces: Vec<&str> = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let close = rest.get(open..)?.find('}')? + open;
        pieces.push(rest.get(..open)?);
        rest = rest.get(close + 1..)?;
    }
    pieces.push(rest);
    let first = pieces.first()?;
    let last = pieces.last()?;
    let mut body = s.strip_prefix(first)?;
    if pieces.len() == 1 {
        return (body.is_empty() && *last == s).then(Vec::new);
    }
    body = body.strip_suffix(last)?;
    let mut parts = Vec::new();
    for middle in pieces.get(1..pieces.len() - 1).unwrap_or_default() {
        let at = body.find(middle)?;
        parts.push(body.get(..at)?);
        body = body.get(at + middle.len()..)?;
    }
    parts.push(body);
    parts.iter().all(|p| !p.is_empty()).then_some(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_give_back_their_parts() {
        assert_eq!(fill("page {0} does not exist", "page 7 does not exist"), Some(vec!["7"]));
        assert_eq!(fill("couldn't use {0}", "couldn't use a.pdf"), Some(vec!["a.pdf"]));
        assert_eq!(
            fill(
                "the standard fonts can't draw \"{0}\" ({1}); only Western European characters can be added as text for now",
                "the standard fonts can't draw \"ก\" (U+0E01); only Western European characters can be added as text for now"
            ),
            Some(vec!["ก", "U+0E01"])
        );
        assert_eq!(fill("page {0} does not exist", "page  does not exist"), None, "an empty part is no match");
        assert_eq!(fill("couldn't use {0}", "something else"), None);
        assert_eq!(os_error("Access is denied. (os error 5)"), Some(5));
        assert_eq!(os_error("no code here"), None);
    }

    /// Every template and OS reason has a Thai row, and reasons read in Thai through them; one
    /// without a row stays in English.
    #[test]
    fn engine_reasons_are_said_in_thai() {
        let th = Lang::from_code("th").expect("th registered");
        for t in TEMPLATES.iter().copied().chain(OS_ERRORS.iter().map(|(_, r)| *r)) {
            assert_ne!(tr_ctx(th, CONTEXT, t), t, "no Thai for {t:?}");
        }
        assert_eq!(detail(th, "nothing to undo"), "ไม่มีอะไรให้ย้อนกลับ");
        assert_eq!(
            detail(th, "the document's security settings don't allow page changes; open it with the owner password to make this change"),
            "การตั้งค่าความปลอดภัยของเอกสารไม่อนุญาตให้เปลี่ยนแปลงหน้า ให้เปิดด้วยรหัสผ่านเจ้าของเอกสารก่อนจึงจะแก้ได้"
        );
        assert_eq!(detail(th, "page 9 does not exist"), "ไม่มีหน้า 9");
        assert_eq!(
            detail(th, "The process cannot access the file because it is being used by another process. (os error 32)"),
            "ไฟล์นี้เปิดอยู่ในโปรแกรมอื่น ให้ปิดโปรแกรมนั้นก่อนแล้วลองอีกครั้ง"
        );
        assert_eq!(detail(th, "a reason with no translation"), "a reason with no translation");
        assert_eq!(detail(Lang::EN, "nothing to undo"), "nothing to undo", "English stays English");
        let ja = Lang::from_code("ja").expect("ja registered");
        assert_eq!(detail(ja, "a document must keep at least one page"), "a document must keep at least one page", "only Thai is translated");
    }
}
