//! What the Source view draws while something is dragged (#264).
//!
//! The Source view is a 4× composite (`Session::PREVIEW_N`); in the browser it costs
//! about 60 ms, too much for every frame of a drag. So while a drag runs:
//!
//! - **A transform drag** (the Nudge or Transform tool, the Transform fields, held arrow
//!   keys) moves cached 4× textures on the GPU: the layers below the dragged one, the
//!   dragged layer by itself (drawn through the drag's affine map) and the layers above.
//! - **Any other drag** (a slider, a field) shows the composite the solve reads (1×),
//!   upscaled with nearest filtering.
//!
//! On release the 4× view is composited once. The solve is not affected.
//!
//! This module holds the logic only, so it can be tested without egui: which drag is in
//! progress ([`DragState`]) and which picture the Source view asks for ([`SourceRequest`]).

use convert64_core::Blend;
use convert64_core::layer::{Layer, Transform};
use convert64_core::mask::SnapToCells;
use convert64_project::{JsonCommand, LayerRef};

/// What a drag changes, as far as the Source view is concerned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DragKind {
    /// Only layer `layer`'s transform; `before` is its transform when the drag started.
    Transform {
        /// The dragged layer (stack index, bottom first).
        layer: usize,
        /// Its transform before the drag's first command.
        before: Transform,
    },
    /// Anything else: a slider or a field.
    Other,
}

/// How a command arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// As part of a drag gesture (one undo step per drag).
    Gesture,
    /// From an arrow key that is still held: its repeats are one drag for the view.
    HeldKey,
    /// By itself (a click, a typed value, a key tapped and released).
    Single,
}

/// A drag in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drag {
    /// What it changes.
    pub kind: DragKind,
    /// The document's revision before its first command.
    pub start: u64,
    /// The revision after its last command: an edit from anywhere else ends the drag.
    last: u64,
}

/// The drag in progress, if any, kept up to date by every edit.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DragState {
    drag: Option<Drag>,
}

/// The layer a transform command moves, if it only moves one (by stack index).
fn moved_layer(command: &JsonCommand) -> Option<usize> {
    match command {
        JsonCommand::LayerSetTransform {
            layer: LayerRef::Index(i),
            ..
        }
        | JsonCommand::LayerNudge {
            layer: LayerRef::Index(i),
            ..
        }
        | JsonCommand::LayerRotate {
            layer: LayerRef::Index(i),
            ..
        } => Some(*i),
        _ => None,
    }
}

impl DragState {
    /// Notes `command`, about to run on a document at revision `rev` whose layers are
    /// `layers`. A gesture's or a held key's command starts a drag or continues it (a
    /// drag that changes more than one layer's transform becomes [`DragKind::Other`]);
    /// any other command ends it.
    pub fn command(&mut self, command: &JsonCommand, via: Via, layers: &[Layer], rev: u64) {
        if via == Via::Single {
            self.drag = None;
            return;
        }
        let kind = match moved_layer(command).and_then(|i| Some((i, layers.get(i)?))) {
            Some((layer, l)) => DragKind::Transform {
                layer,
                before: l.transform(),
            },
            None => DragKind::Other,
        };
        match &mut self.drag {
            Some(d) if d.last == rev => {
                let same = match (d.kind, kind) {
                    (
                        DragKind::Transform { layer: a, .. },
                        DragKind::Transform { layer: b, .. },
                    ) => a == b,
                    (DragKind::Other, DragKind::Other) => true,
                    _ => false,
                };
                if !same {
                    d.kind = DragKind::Other;
                }
            }
            _ => {
                self.drag = Some(Drag {
                    kind,
                    start: rev,
                    last: rev,
                });
            }
        }
    }

    /// The command noted last ran; the document is now at revision `rev`.
    pub fn ran(&mut self, rev: u64) {
        if let Some(d) = &mut self.drag {
            d.last = rev;
        }
    }

    /// The drag ended: released or cancelled. The app also ends it on any frame with
    /// neither a pointer button nor an arrow key down, so a gesture without a release
    /// (a wheel notch scaling a layer) never leaves the view coarse.
    pub fn end(&mut self) {
        self.drag = None;
    }

    /// The drag in progress on a document at revision `rev`: none if an edit from
    /// outside the drag (an undo, a loaded palette) ran since its last command.
    pub fn current(&self, rev: u64) -> Option<&Drag> {
        self.drag.as_ref().filter(|d| d.last == rev)
    }
}

/// The picture the Source view asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SourceRequest {
    /// The 4× composite (no drag, or the drag's release).
    Full,
    /// The solve's composite (1×), upscaled.
    Coarse,
    /// Cached 4× textures moved on the GPU: layer `layer` through the drag's affine map
    /// over the layers below, under the layers above. `before` is its transform when the
    /// drag started.
    Moving {
        /// The dragged layer.
        layer: usize,
        /// Its transform before the drag.
        before: Transform,
    },
}

/// What the Source view asks for during `drag` over `layers`.
///
/// A moved layer is drawn on the GPU only where that is the composite: laying textures
/// over each other is the compositor's Normal blend at full opacity (and on hard edges,
/// where the GPU blending in sRGB rather than linear light makes no visible difference).
/// So the dragged layer and every visible layer above it must be Normal and opaque; the
/// layers above must have no mask (their soft edges would blend in sRGB over the moving
/// layer), and the dragged layer no mask over visible layers below for the same reason,
/// and no mask snapped to cells (that mask does not move rigidly with its layer).
/// Anything else falls back to [`SourceRequest::Coarse`], which is always exact.
pub fn request(drag: Option<&Drag>, layers: &[Layer]) -> SourceRequest {
    let Some(drag) = drag else {
        return SourceRequest::Full;
    };
    let DragKind::Transform { layer, before } = drag.kind else {
        return SourceRequest::Coarse;
    };
    let Some(l) = layers.get(layer) else {
        return SourceRequest::Coarse;
    };
    let opaque_normal = |l: &Layer| l.blend() == Blend::Normal && l.opacity() >= 1.0;
    let below = layers[..layer].iter().any(Layer::visible);
    let own_mask = l
        .mask()
        .is_some_and(|m| below || m.modifiers().snap() != SnapToCells::Off);
    let above_ok = layers[layer + 1..]
        .iter()
        .filter(|a| a.visible())
        .all(|a| opaque_normal(a) && a.mask().is_none());
    if opaque_normal(l) && !own_mask && above_ok {
        SourceRequest::Moving { layer, before }
    } else {
        SourceRequest::Coarse
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use convert64_core::Vec2;
    use convert64_core::layer::SourceImage;
    use convert64_core::mask::{Mask, Modifiers};
    use std::sync::Arc;

    fn layer() -> Layer {
        let src = SourceImage::from_srgba8(4, 4, &[128; 4 * 4 * 4]).unwrap();
        Layer::new("l", Arc::new(src))
    }

    fn nudge(i: usize) -> JsonCommand {
        JsonCommand::LayerNudge {
            layer: LayerRef::Index(i),
            by: Vec2::new(1.0, 0.0),
        }
    }

    fn opacity(i: usize) -> JsonCommand {
        JsonCommand::LayerSetOpacity {
            layer: LayerRef::Index(i),
            opacity: 0.5,
        }
    }

    /// Notes and "runs" `command` on a document at `rev`, as the workspace does.
    fn step(s: &mut DragState, command: &JsonCommand, via: Via, layers: &[Layer], rev: &mut u64) {
        s.command(command, via, layers, *rev);
        *rev += 1;
        s.ran(*rev);
    }

    #[test]
    fn a_nudge_drag_moves_textures_and_its_release_composites_in_full() {
        let layers = vec![layer(), layer()];
        let mut s = DragState::default();
        let mut rev = 3;
        assert_eq!(request(s.current(rev), &layers), SourceRequest::Full);
        for _ in 0..5 {
            step(&mut s, &nudge(1), Via::Gesture, &layers, &mut rev);
            let before = layers[1].transform();
            assert_eq!(
                request(s.current(rev), &layers),
                SourceRequest::Moving { layer: 1, before }
            );
        }
        assert_eq!(s.current(rev).map(|d| d.start), Some(3));
        s.end();
        assert_eq!(request(s.current(rev), &layers), SourceRequest::Full);
    }

    #[test]
    fn a_slider_drag_shows_the_coarse_composite() {
        let layers = vec![layer()];
        let mut s = DragState::default();
        let mut rev = 0;
        step(&mut s, &opacity(0), Via::Gesture, &layers, &mut rev);
        assert_eq!(request(s.current(rev), &layers), SourceRequest::Coarse);
        s.end();
        assert_eq!(request(s.current(rev), &layers), SourceRequest::Full);
    }

    #[test]
    fn a_single_edit_or_an_edit_from_elsewhere_ends_the_drag() {
        let layers = vec![layer()];
        let mut s = DragState::default();
        let mut rev = 0;
        step(&mut s, &nudge(0), Via::Gesture, &layers, &mut rev);
        assert!(s.current(rev).is_some());
        // An undo bumps the revision without going through the drag.
        assert_eq!(s.current(rev + 1), None);
        step(&mut s, &nudge(0), Via::Single, &layers, &mut rev);
        assert_eq!(request(s.current(rev), &layers), SourceRequest::Full);
    }

    #[test]
    fn held_arrow_keys_are_one_drag_until_released() {
        let layers = vec![layer()];
        let mut s = DragState::default();
        let mut rev = 0;
        for _ in 0..4 {
            step(&mut s, &nudge(0), Via::HeldKey, &layers, &mut rev);
            assert!(matches!(
                request(s.current(rev), &layers),
                SourceRequest::Moving { layer: 0, .. }
            ));
        }
        assert_eq!(s.current(rev).map(|d| d.start), Some(0));
        s.end();
        assert_eq!(request(s.current(rev), &layers), SourceRequest::Full);
    }

    #[test]
    fn a_drag_that_changes_more_than_one_layer_is_coarse() {
        let layers = vec![layer(), layer()];
        let mut s = DragState::default();
        let mut rev = 0;
        step(&mut s, &nudge(0), Via::Gesture, &layers, &mut rev);
        step(&mut s, &nudge(1), Via::Gesture, &layers, &mut rev);
        assert_eq!(request(s.current(rev), &layers), SourceRequest::Coarse);
        step(&mut s, &nudge(0), Via::Gesture, &layers, &mut rev);
        assert_eq!(request(s.current(rev), &layers), SourceRequest::Coarse);
    }

    fn moving(layers: &[Layer], i: usize) -> bool {
        let mut s = DragState::default();
        s.command(&nudge(i), Via::Gesture, layers, 0);
        s.ran(1);
        matches!(request(s.current(1), layers), SourceRequest::Moving { .. })
    }

    #[test]
    fn opacity_blend_modes_and_soft_edges_fall_back_to_the_coarse_composite() {
        let plain = vec![layer(), layer(), layer()];
        assert!(moving(&plain, 0) && moving(&plain, 1) && moving(&plain, 2));

        // The dragged layer translucent or not Normal.
        let mut l = plain.clone();
        l[1].set_opacity(0.5).unwrap();
        assert!(!moving(&l, 1));
        assert!(!moving(&l, 0), "a translucent layer above");
        assert!(moving(&l, 2), "below the moved layer, anything goes");
        let mut l = plain.clone();
        l[1].set_blend(Blend::Multiply);
        assert!(!moving(&l, 1) && !moving(&l, 0) && moving(&l, 2));
        // Hidden layers above do not count.
        l[1].set_visible(false);
        assert!(moving(&l, 0));

        // A mask: fine on the bottom layer, not over visible layers or above.
        let mask = || Some(Mask::opaque(4, 4).unwrap());
        let mut l = plain.clone();
        l[0].set_mask(mask()).unwrap();
        assert!(moving(&l, 0));
        l[1].set_mask(mask()).unwrap();
        assert!(!moving(&l, 1), "a mask over a visible layer below");
        assert!(!moving(&l, 0), "a mask above");
        l[0].set_visible(false);
        assert!(moving(&l, 1), "the layer below is hidden");
        // A mask snapped to cells does not move with its layer.
        let mut l = plain.clone();
        let mut m = Mask::opaque(4, 4).unwrap();
        m.set_modifiers(Modifiers::default().with_snap(SnapToCells::Expand));
        l[0].set_mask(Some(m)).unwrap();
        assert!(!moving(&l, 0));
    }
}
