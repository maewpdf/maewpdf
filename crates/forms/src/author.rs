//! Prepare a form: create, change and delete fields (execution plan M6.6).
//!
//! New fields follow Acrobat's conventions: names "Text1", "Check Box1", "Group1" (radio groups),
//! "Dropdown1", "List Box1", "Button1", "Signature1", "Date1"; Helvetica at Acrobat's Auto size
//! for text and date fields (12 pt in a box that has room, smaller in a short one), 12 pt in
//! choice fields, a thin grey border and a white background, and appearance streams generated
//! immediately so every viewer shows the empty field.

use pdfcraft_cos::{Dict, Document, ObjRef, Object, PdfString, Stream};

use crate::{Field, FieldKind, FormError, Widget, appearance, fields, flags, page_refs};

mod edit;
mod join;
mod lifecycle;
mod look;
mod tree;

pub(crate) use tree::ancestor_sets;

pub use edit::set_props;
pub(crate) use edit::set_props_object;
pub use lifecycle::{add_field, copy_field, delete_field, delete_field_object, delete_widget, duplicate_field};
pub(crate) use look::button_appearance;
pub(crate) use look::redraw_field_object;
pub use look::{
    BorderStyle, CheckStyle, FieldFont, Look, LookPatch, check_style, look, redraw_field, redraw_fields, set_button_icon, set_button_icon_target,
};
pub(crate) use tree::{ensure_form, form_fonts_as_objects};

/// The smallest width and height of a field's widget, in points: a smaller one is refused.
pub const MIN_FIELD_SIDE: f64 = 4.0;

/// The kind of field to add.
#[derive(Clone, Debug, PartialEq)]
pub enum NewField {
    Text {
        multiline: bool,
    },
    /// A date field: a text field with Acrobat's date format action (`AFDate_FormatEx`).
    Date,
    CheckBox,
    /// One radio button in `group` (a new group is created when none of that name exists),
    /// selected by `export`.
    Radio {
        group: Option<String>,
        export: String,
    },
    Combo {
        options: Vec<String>,
        editable: bool,
    },
    List {
        options: Vec<String>,
        multi: bool,
    },
    Button {
        caption: String,
    },
    /// An image field: an icon-only push button that asks for a picture when clicked.
    Image,
    Signature,
}

impl NewField {
    fn base_name(&self) -> &'static str {
        match self {
            NewField::Text { .. } => "Text",
            NewField::Date => "Date",
            NewField::CheckBox => "Check Box",
            NewField::Radio { .. } => "Group",
            NewField::Combo { .. } => "Dropdown",
            NewField::List { .. } => "List Box",
            NewField::Button { .. } => "Button",
            NewField::Image => "Image",
            NewField::Signature => "Signature",
        }
    }
}

/// How a field shows on screen and on paper (Field Properties ▸ General ▸ Form field), from
/// its widgets' annotation flags Hidden (2), Print (4) and NoView (32), ISO 32000-2 Table 167.
/// The same four states as a script's `field.display`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FieldDisplay {
    /// Shown and printed.
    #[default]
    Visible,
    /// Neither shown nor printed.
    Hidden,
    /// Shown, not printed.
    NoPrint,
    /// Printed, not shown.
    NoView,
}

impl FieldDisplay {
    pub const ALL: [FieldDisplay; 4] = [FieldDisplay::Visible, FieldDisplay::Hidden, FieldDisplay::NoPrint, FieldDisplay::NoView];

    /// The state a widget's `/F` flags give.
    pub fn of(flags: i64) -> FieldDisplay {
        if flags & 2 != 0 {
            FieldDisplay::Hidden
        } else if flags & 32 != 0 {
            FieldDisplay::NoView
        } else if flags & 4 == 0 {
            FieldDisplay::NoPrint
        } else {
            FieldDisplay::Visible
        }
    }

    /// `flags` with this state's Hidden, Print and NoView bits, the other bits kept.
    pub fn flags(self, flags: i64) -> i64 {
        let base = flags & !(2 | 4 | 32);
        match self {
            FieldDisplay::Visible => base | 4,
            FieldDisplay::Hidden => base | 2 | 4,
            FieldDisplay::NoPrint => base,
            FieldDisplay::NoView => base | 4 | 32,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FieldDisplay::Visible => "Visible",
            FieldDisplay::Hidden => "Hidden",
            FieldDisplay::NoPrint => "Visible but doesn't print",
            FieldDisplay::NoView => "Hidden but printable",
        }
    }
}

/// A field's [`FieldDisplay`], as its first widget's flags say (no flags: Hidden, Print and
/// NoView all clear, so shown but not printed).
pub fn field_display(doc: &Document, f: &Field) -> FieldDisplay {
    let flags = f.widgets.first().and_then(|w| doc.get(w.obj).as_dict().and_then(|d| d.get(b"F").and_then(|v| doc.resolve(v).as_int()))).unwrap_or(0);
    FieldDisplay::of(flags)
}

/// Field properties the Properties dialog edits (`None` leaves a value unchanged).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FieldProps {
    pub name: Option<String>,
    pub tooltip: Option<String>,
    /// Normal push-button label (`/MK /CA`) of one widget (0-based index).
    /// An empty string clears the label; other widgets and icon layouts are preserved.
    pub caption: Option<(usize, String)>,
    /// Normal push-button label for every widget in the field.
    pub caption_all: Option<String>,
    pub read_only: Option<bool>,
    pub required: Option<bool>,
    pub multiline: Option<bool>,
    pub max_len: Option<Option<usize>>,
    /// Choice options (display texts; export values equal them).
    pub options: Option<Vec<String>>,
    /// Change one button widget's export value without changing its appearance state.
    /// Widgets sharing that state are kept in sync. The index is zero-based.
    pub export_value: Option<(usize, String)>,
    /// Font size (0 = auto).
    pub font_size: Option<f64>,
    /// Position: move or resize one widget (its index in [`Field::widgets`]) to a new rect in
    /// user space.
    pub rect: Option<(usize, [f64; 4])>,
    /// Appearance tab (border, fill, line, text colour, font).
    pub look: Option<Look>,
    /// Change only these appearance properties. Unlike `look`, this preserves each widget's
    /// other colours, border settings and default appearance, including custom font resources.
    pub appearance: Option<LookPatch>,
    /// Format, Validate and Calculate tabs (written as Acrobat's AF scripts).
    pub format: Option<crate::af::Format>,
    pub validate: Option<crate::af::Validate>,
    pub calculate: Option<crate::af::Calculate>,
    /// Options-tab flags (`/Ff` bits from [`crate::flags`]) to set or clear.
    pub flags: Vec<(u32, bool)>,
    /// Alignment (`/Q`): 0 left, 1 centre, 2 right.
    pub quadding: Option<i64>,
    /// Default value (`/DV`): what Reset form restores.
    pub default_value: Option<Option<String>>,
    /// Lock or unlock the field (its widgets' Locked flag).
    pub locked: Option<bool>,
    /// Actions tab: every trigger's action (replacing the field's).
    pub actions: Option<Vec<(crate::Trigger, crate::FieldAction)>>,
    /// Options tab: the mark of a check box or radio button (every widget).
    pub check_style: Option<CheckStyle>,
    /// Position tab: rotate one widget (`/MK /R`) to 0, 90, 180 or 270 degrees counterclockwise.
    /// The widget keeps its rectangle: `/R` turns the appearance inside it (ISO 32000-2 Table 192
    /// turns the annotation relative to the page, and its `/Rect` stays in default user space), so
    /// a tall field turned 90 degrees holds text running up its long side.
    pub rotation: Option<(usize, i64)>,
    /// General tab: shown and printed, hidden, or one of the two (every widget's `/F`).
    pub display: Option<FieldDisplay>,
}

/// A [`FormError::Invalid`] with message `m`.
fn invalid<T>(m: impl Into<String>) -> Result<T, FormError> {
    Err(FormError::Invalid(m.into()))
}

/// Put `value` into field `f` as its kind takes one (a text value through the field's Keystroke
/// and Validate events and its scripts first), and redraw the field.
fn rgb(c: [f64; 3]) -> Object {
    Object::Array(c.iter().map(|v| Object::Real(*v)).collect())
}
