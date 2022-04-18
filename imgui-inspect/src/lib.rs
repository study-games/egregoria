mod default;
mod scale;
mod slider;

pub use default::*;
pub use imgui;
pub use scale::*;
pub use slider::*;

/// Options for rendering a value as a struct (i.e. draw all of its subfields)
#[derive(Default, Debug)]
pub struct InspectArgsStruct {
    pub header: Option<bool>,
    pub indent_children: Option<bool>,
}

impl From<InspectArgsDefault> for InspectArgsStruct {
    fn from(default_args: InspectArgsDefault) -> Self {
        Self {
            header: default_args.header,
            indent_children: default_args.indent_children,
        }
    }
}

/// Renders a struct (i.e. draw all of its subfields). Most traits are implemented by hand-written code, but this trait
/// is normally generated by putting `#[derive(Inspect)]` on a struct
pub trait InspectRenderStruct<T> {
    fn render(data: &[&T], label: &'static str, ui: &imgui::Ui<'_>, args: &InspectArgsStruct);
    fn render_mut(
        data: &mut [&mut T],
        label: &'static str,
        ui: &imgui::Ui<'_>,
        args: &InspectArgsStruct,
    ) -> bool;
}

/// Utility function that, given a list of references, returns Some(T) if they are the same, otherwise None
pub fn get_same_or_none<'a, T: PartialEq + Clone>(data: &'a [&T]) -> Option<&'a T> {
    if data.is_empty() {
        return None;
    }

    if data.len() == 1 {
        return Some(data[0]);
    }

    let first = data[0];
    for d in data {
        if *d != first {
            return None;
        }
    }

    Some(first)
}

/// Utility function that, given a list of references, returns Some(T) if they are the same, otherwise None
fn get_same_or_none_mut<T: PartialEq + Clone>(data: &mut [&mut T]) -> Option<T> {
    if data.is_empty() {
        return None;
    }

    let first = data[0].clone();
    for d in data {
        if **d != first {
            return None;
        }
    }

    Some(first)
}

#[rustfmt::skip]
#[macro_export]
macro_rules! debug_inspect_impl {
    ($t: ty) => {
        impl imgui_inspect::InspectRenderDefault<$t> for $t {
            fn render(
                data: &[&$t],
                label: &'static str,
                ui: &imgui_inspect::imgui::Ui,
                _: &imgui_inspect::InspectArgsDefault,
            ) {
                let d = match data.get(0) { Some(x) => x, None => return };
                if label == "" {
                    ui.text(imgui_inspect::imgui::im_str!("{:?}", d));
                } else {
                    ui.text(imgui_inspect::imgui::im_str!("{}: {:?}", label, d));
                }
            }

            fn render_mut(
                data: &mut [&mut $t],
                label: &'static str,
                ui: &imgui_inspect::imgui::Ui,
                _: &imgui_inspect::InspectArgsDefault,
            ) -> bool {
                let d = match data.get(0) { Some(x) => x, None => return false };
                ui.text(imgui_inspect::imgui::im_str!("{}: {:?}", label, d));
                false
            }
        }
    };
}
