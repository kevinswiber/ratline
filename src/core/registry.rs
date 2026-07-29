//! The pure declaration every entry point constructs: what each source
//! runs, how often, and the box it paints into. No threads, no
//! processes, no terminal reads, no `#[cfg]` — the runtime resources
//! (child slots, schedules, trigger readers) live in the engine, keyed
//! by [`SourceId`].

// The model lands before the engine that reads it; the dashboard
// subcommand is the first real caller and removes this.
#![allow(dead_code)]

use std::time::Duration;

use anyhow::bail;

use crate::core::box_model::{BorderPreset, Sides};
use crate::core::trigger::TriggerSpec;

/// Stable index of one source: the tag on every outcome and the key of
/// every per-source runtime resource.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct SourceId(pub usize);

/// What one source runs and how often — what every surface constructs.
#[derive(Clone, PartialEq, Debug)]
pub struct SourceSpec {
    pub name: String,
    /// argv when `shell` is false; the single script string (joined
    /// with spaces, exactly as `rat watch --shell` does) when it is
    /// true.
    pub command: Vec<String>,
    pub shell: bool,
    /// `None`: no deadline of its own, triggers only.
    pub interval: Option<Duration>,
    pub triggers: Vec<TriggerSpec>,
    pub debounce: Duration,
}

/// The declared box a source paints into. Height is PINNED: the
/// composed frame's row count is run-constant, which is what keeps the
/// retained-row differ on its cheap path.
#[derive(Clone, PartialEq, Debug)]
pub struct PaneBox {
    /// The finished box, borders and chrome included.
    pub height: u16,
    pub width: PaneWidth,
    pub overflow: Overflow,
    pub border: BorderPreset,
    pub padding: Sides,
    /// `None` renders the source's name.
    pub title: Option<String>,
    /// The faint cadence/freshness row, the last interior row.
    pub chrome: bool,
}

impl PaneBox {
    /// Rows and cells one border edge consumes: the preset decides, so
    /// a future preset that draws nothing stays free.
    pub fn edge_cells(&self) -> u16 {
        u16::from(self.border.set().is_some())
    }

    /// Everything in `height` that is not the child's: both borders,
    /// the vertical padding, and the status row.
    pub fn frame_rows(&self) -> u16 {
        let padding = self.padding.top.saturating_add(self.padding.bottom);
        self.edge_cells()
            .saturating_mul(2)
            .saturating_add(padding.min(u16::MAX as usize) as u16)
            .saturating_add(u16::from(self.chrome))
    }

    /// Everything in the pane's cells that is not the child's: both
    /// borders and the horizontal padding.
    pub fn frame_cols(&self) -> u16 {
        let padding = self.padding.left.saturating_add(self.padding.right);
        self.edge_cells()
            .saturating_mul(2)
            .saturating_add(padding.min(u16::MAX as usize) as u16)
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum PaneWidth {
    Weight(u16),
    Cells(u16),
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Overflow {
    /// Dashboards lead with the headline: longer content drops its tail.
    #[default]
    KeepTop,
    /// A log tail keeps the bottom instead.
    KeepBottom,
}

/// A vertical stack of rows; a row is one or more panes joined
/// horizontally. Nesting is representable so a future grid needs no
/// re-model; the v1 file grammars construct depth two only.
#[derive(Clone, PartialEq, Debug)]
pub enum LayoutNode {
    Pane(SourceId),
    Row(Vec<LayoutNode>),
    Column(Vec<LayoutNode>),
}

/// How drained outputs become one frame.
#[derive(Clone, PartialEq, Debug)]
pub enum Composition {
    /// `rat watch`: the shipped compose — title, stdout, stderr. No
    /// geometry, no boxes, byte-frozen.
    Plain { title: Option<String> },
    /// `rat dashboard`: declared boxes composed by the layout tree.
    Panes {
        layout: LayoutNode,
        gap: usize,
        row_gap: usize,
    },
}

/// The whole declaration the loop runs. The index into `sources` and
/// `panes` IS the `SourceId` — which is why the length check below is a
/// hard error rather than a zip that silently truncates.
#[derive(Clone, Debug)]
pub struct Registry {
    sources: Vec<SourceSpec>,
    /// Empty under `Composition::Plain`: watch declares no box.
    panes: Vec<PaneBox>,
    composition: Composition,
}

/// One pane's resolved box for the current terminal width.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct PaneGeometry {
    /// The whole box.
    pub cells: u16,
    /// == `PaneBox::height`.
    pub rows: u16,
    /// Handed to the child as `RAT_WIDTH`.
    pub inner_cols: u16,
    /// Handed to the child as `RAT_HEIGHT`; EXCLUDES the chrome row —
    /// the loop owns that row, not the child.
    pub inner_rows: u16,
}

impl Registry {
    /// The N == 1 constructor `rat watch` uses: one source, no box.
    pub fn single(spec: SourceSpec, title: Option<String>) -> Registry {
        Registry {
            sources: vec![spec],
            panes: Vec::new(),
            composition: Composition::Plain { title },
        }
    }

    /// The N-pane constructor. Every way a declaration can be
    /// incoherent fails HERE, before a single child spawns.
    pub fn panes(
        sources: Vec<SourceSpec>,
        panes: Vec<PaneBox>,
        layout: LayoutNode,
        gap: usize,
        row_gap: usize,
    ) -> anyhow::Result<Registry> {
        if sources.len() != panes.len() {
            bail!(
                "{} sources declared but {} panes: every source paints into exactly one pane",
                sources.len(),
                panes.len()
            );
        }
        let mut placed = vec![0usize; sources.len()];
        count_placements(&layout, &mut placed, sources.len())?;
        for (source, count) in sources.iter().zip(&placed) {
            match count {
                0 => bail!(
                    "pane {:?} is declared but never placed in the layout",
                    source.name
                ),
                1 => {}
                n => bail!(
                    "pane {:?} is placed {n} times in the layout; place it once",
                    source.name
                ),
            }
        }
        for (source, pane) in sources.iter().zip(&panes) {
            let frame = pane.frame_rows();
            if pane.height <= frame {
                bail!(
                    "pane {:?} is {} rows tall, but its border, padding, and status row \
                     already take {frame}: give it at least {}",
                    source.name,
                    pane.height,
                    frame + 1
                );
            }
        }
        Ok(Registry {
            sources,
            panes,
            composition: Composition::Panes {
                layout,
                gap,
                row_gap,
            },
        })
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// Declaration order — the combining hash and the runtime vec are
    /// both index-keyed, so this order is a contract.
    pub fn ids(&self) -> impl Iterator<Item = SourceId> + '_ {
        (0..self.sources.len()).map(SourceId)
    }

    pub fn spec(&self, id: SourceId) -> &SourceSpec {
        &self.sources[id.0]
    }

    pub fn pane(&self, id: SourceId) -> Option<&PaneBox> {
        self.panes.get(id.0)
    }

    pub fn composition(&self) -> &Composition {
        &self.composition
    }

    /// Per-pane geometry for one terminal size, resolved BEFORE the
    /// spawn step so a child can be told its pane's inner size.
    pub fn geometry(&self, size: (u16, u16)) -> Vec<PaneGeometry> {
        let mut out = vec![
            PaneGeometry {
                cells: 0,
                rows: 0,
                inner_cols: 0,
                inner_rows: 0,
            };
            self.sources.len()
        ];
        match &self.composition {
            // Watch's shipped contract: the child is told the terminal
            // size verbatim (RAT_WIDTH/RAT_HEIGHT must not move). Do
            // not "simplify" this arm into the pane path.
            Composition::Plain { .. } => {
                out.fill(PaneGeometry {
                    cells: size.0,
                    rows: size.1,
                    inner_cols: size.0,
                    inner_rows: size.1,
                });
            }
            Composition::Panes { layout, gap, .. } => {
                self.allocate(layout, size.0, *gap, &mut out);
            }
        }
        out
    }

    fn allocate(&self, node: &LayoutNode, cells: u16, gap: usize, out: &mut [PaneGeometry]) {
        match node {
            LayoutNode::Pane(id) => {
                if let (Some(pane), Some(geom)) = (self.panes.get(id.0), out.get_mut(id.0)) {
                    *geom = PaneGeometry {
                        cells,
                        rows: pane.height,
                        inner_cols: cells.saturating_sub(pane.frame_cols()),
                        inner_rows: pane.height.saturating_sub(pane.frame_rows()),
                    };
                }
            }
            // A column's children each own the full width they were given.
            LayoutNode::Column(children) => {
                for child in children {
                    self.allocate(child, cells, gap, out);
                }
            }
            LayoutNode::Row(children) => {
                let widths: Vec<PaneWidth> = children.iter().map(|c| self.width_of(c)).collect();
                for (child, share) in children.iter().zip(allocate_row(cells, gap, &widths)) {
                    self.allocate(child, share, gap, out);
                }
            }
        }
    }

    /// A row splits by its children's declared widths; a nested row or
    /// column declares none of its own and takes an equal share.
    fn width_of(&self, node: &LayoutNode) -> PaneWidth {
        match node {
            LayoutNode::Pane(id) => self
                .panes
                .get(id.0)
                .map(|pane| pane.width)
                .unwrap_or(PaneWidth::Weight(1)),
            _ => PaneWidth::Weight(1),
        }
    }
}

/// Tally how often each source appears in the layout, refusing an id
/// the declaration never made.
fn count_placements(
    node: &LayoutNode,
    placed: &mut [usize],
    declared: usize,
) -> anyhow::Result<()> {
    match node {
        LayoutNode::Pane(id) => {
            let Some(slot) = placed.get_mut(id.0) else {
                bail!(
                    "the layout places pane index {}, but only {declared} panes are declared",
                    id.0
                );
            };
            *slot += 1;
        }
        LayoutNode::Row(children) | LayoutNode::Column(children) => {
            for child in children {
                count_placements(child, placed, declared)?;
            }
        }
    }
    Ok(())
}

/// No pane shrinks below this floor. A row whose declared widths cannot
/// all reach it overflows the terminal on purpose: a chopped right edge
/// is legible, a pane silently shrunk to nothing is not.
pub const MIN_PANE_CELLS: u16 = 8;

/// Split a row's cells: `Cells` panes take their own, the remainder is
/// shared by weight, flooring leftovers go left to right, nothing below
/// [`MIN_PANE_CELLS`]. The floor is applied last so it can never be
/// undone.
pub fn allocate_row(total: u16, gap: usize, widths: &[PaneWidth]) -> Vec<u16> {
    if widths.is_empty() {
        return Vec::new();
    }
    let gaps = gap.saturating_mul(widths.len() - 1);
    let usable = (total as usize).saturating_sub(gaps);
    let fixed: usize = widths.iter().map(|w| declared_cells(*w)).sum();
    let weights: usize = widths.iter().map(|w| declared_weight(*w)).sum();
    let pool = usable.saturating_sub(fixed);

    let mut cells: Vec<usize> = Vec::with_capacity(widths.len());
    for width in widths {
        cells.push(match width {
            PaneWidth::Cells(_) => declared_cells(*width),
            // Floor now; the remainder is handed out below, so no cell
            // is lost to rounding.
            PaneWidth::Weight(_) if weights > 0 => pool * declared_weight(*width) / weights,
            PaneWidth::Weight(_) => 0,
        });
    }
    let spent: usize = cells.iter().sum();
    let mut leftover = usable.saturating_sub(spent);
    for (slot, width) in cells.iter_mut().zip(widths) {
        if leftover == 0 {
            break;
        }
        if matches!(width, PaneWidth::Weight(_)) {
            *slot += 1;
            leftover -= 1;
        }
    }
    cells
        .into_iter()
        .map(|c| c.clamp(MIN_PANE_CELLS as usize, u16::MAX as usize) as u16)
        .collect()
}

/// A `Cells` pane's own cells, never below the floor; zero for a
/// weighted one.
fn declared_cells(width: PaneWidth) -> usize {
    match width {
        PaneWidth::Cells(c) => c.max(MIN_PANE_CELLS) as usize,
        PaneWidth::Weight(_) => 0,
    }
}

/// A weighted pane's share; zero for a fixed one. Weight 0 reads as 1 —
/// a pane the declaration placed is a pane the user wants to see.
fn declared_weight(width: PaneWidth) -> usize {
    match width {
        PaneWidth::Weight(k) => k.max(1) as usize,
        PaneWidth::Cells(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str) -> SourceSpec {
        SourceSpec {
            name: name.to_string(),
            command: vec!["true".to_string()],
            shell: false,
            interval: Some(Duration::from_secs(2)),
            triggers: Vec::new(),
            debounce: Duration::from_millis(120),
        }
    }

    /// A rounded, `"0 1"`-padded pane with the status row — the shape the
    /// example dashboards declare.
    fn pane(height: u16, width: PaneWidth) -> PaneBox {
        PaneBox {
            height,
            width,
            overflow: Overflow::default(),
            border: BorderPreset::Rounded,
            padding: Sides {
                top: 0,
                right: 1,
                bottom: 0,
                left: 1,
            },
            title: None,
            chrome: true,
        }
    }

    fn stacked(n: usize) -> LayoutNode {
        LayoutNode::Column((0..n).map(|i| LayoutNode::Pane(SourceId(i))).collect())
    }

    #[test]
    fn a_row_splits_its_cells_by_weight_and_fixed_cells() {
        let widths = [
            PaneWidth::Cells(20),
            PaneWidth::Weight(2),
            PaneWidth::Weight(1),
        ];
        let cells = allocate_row(80, 1, &widths);
        // 80 − 2 gaps = 78 usable; 20 fixed; 58 shared 2:1 with the
        // flooring leftover going to the leftmost weighted pane.
        assert_eq!(cells, vec![20, 39, 19]);
        let used: usize = cells.iter().map(|c| *c as usize).sum();
        assert_eq!(used + 2, 80, "the row must account for every cell");
    }

    #[test]
    fn a_fixed_pane_that_cannot_fit_clamps_at_the_minimum() {
        let cells = allocate_row(20, 1, &[PaneWidth::Cells(30), PaneWidth::Weight(1)]);
        // Nothing shrinks below the floor: the row overflows the terminal
        // on purpose and the paint chops the right edge.
        assert_eq!(cells, vec![30, MIN_PANE_CELLS]);
        let used: usize = cells.iter().map(|c| *c as usize).sum();
        assert!(
            used > 20,
            "an unfittable row overflows rather than vanishing"
        );
        // A declared width under the floor is raised to it.
        assert_eq!(
            allocate_row(80, 0, &[PaneWidth::Cells(3), PaneWidth::Weight(1)]),
            vec![MIN_PANE_CELLS, 72]
        );
        assert_eq!(allocate_row(80, 1, &[]), Vec::<u16>::new());
    }

    #[test]
    fn pane_geometry_subtracts_border_padding_and_chrome() {
        let registry = Registry::panes(
            vec![spec("plan")],
            vec![pane(7, PaneWidth::Weight(1))],
            stacked(1),
            1,
            0,
        )
        .unwrap();
        // Rounded border: 2 rows and 2 cells. Padding "0 1": 2 cells.
        // Status row: 1 row. 7 − 3 = 4 content rows; 40 − 4 = 36 cells.
        assert_eq!(
            registry.geometry((40, 24)),
            vec![PaneGeometry {
                cells: 40,
                rows: 7,
                inner_cols: 36,
                inner_rows: 4,
            }]
        );

        // Borderless, unpadded, no status row: the box IS the inner box.
        let bare = PaneBox {
            border: BorderPreset::None,
            padding: Sides::default(),
            chrome: false,
            ..pane(7, PaneWidth::Weight(1))
        };
        let registry = Registry::panes(vec![spec("plan")], vec![bare], stacked(1), 1, 0).unwrap();
        assert_eq!(
            registry.geometry((40, 24)),
            vec![PaneGeometry {
                cells: 40,
                rows: 7,
                inner_cols: 40,
                inner_rows: 7,
            }]
        );
    }

    #[test]
    fn the_plain_composition_hands_the_terminal_size_through() {
        let registry = Registry::single(spec("watch"), Some("build".to_string()));
        assert_eq!(registry.len(), 1);
        assert!(!registry.is_empty());
        assert!(
            registry.pane(SourceId(0)).is_none(),
            "plain declares no box"
        );
        assert!(matches!(registry.composition(), Composition::Plain { .. }));
        // The shipped RAT_WIDTH/RAT_HEIGHT contract, pinned at the model
        // level: one entry, the terminal verbatim, no geometry math (I-51).
        assert_eq!(
            registry.geometry((100, 30)),
            vec![PaneGeometry {
                cells: 100,
                rows: 30,
                inner_cols: 100,
                inner_rows: 30,
            }]
        );
    }

    #[test]
    fn a_layout_naming_an_undeclared_pane_is_rejected() {
        let err = Registry::panes(
            vec![spec("plan")],
            vec![pane(7, PaneWidth::Weight(1))],
            LayoutNode::Row(vec![
                LayoutNode::Pane(SourceId(0)),
                LayoutNode::Pane(SourceId(1)),
            ]),
            1,
            0,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("index 1"), "got {err:?}");
        assert!(err.contains("declared"), "got {err:?}");
    }

    #[test]
    fn a_pane_missing_from_the_layout_is_rejected() {
        let err = Registry::panes(
            vec![spec("plan"), spec("guardrails")],
            vec![pane(7, PaneWidth::Weight(1)), pane(7, PaneWidth::Weight(1))],
            stacked(1),
            1,
            0,
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("guardrails"),
            "the error names the pane: {err:?}"
        );
        assert!(err.contains("layout"), "and where it is missing: {err:?}");
    }

    #[test]
    fn a_pane_placed_twice_is_rejected() {
        // Identity is the index, so a pane in two rows has two geometries
        // and one output — the ambiguity dies at construction.
        let err = Registry::panes(
            vec![spec("git")],
            vec![pane(7, PaneWidth::Weight(1))],
            LayoutNode::Column(vec![
                LayoutNode::Pane(SourceId(0)),
                LayoutNode::Pane(SourceId(0)),
            ]),
            1,
            0,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("git"), "got {err:?}");
    }

    #[test]
    fn sources_and_panes_must_line_up() {
        let err = Registry::panes(
            vec![spec("plan"), spec("git")],
            vec![pane(7, PaneWidth::Weight(1))],
            stacked(2),
            1,
            0,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains('2') && err.contains('1'), "got {err:?}");
    }

    #[test]
    fn a_height_below_its_chrome_is_rejected_and_names_the_pane() {
        // Rounded border (2 rows) + status row (1) leaves nothing for the
        // child at height 3.
        let err = Registry::panes(
            vec![spec("guardrails")],
            vec![pane(3, PaneWidth::Weight(1))],
            stacked(1),
            1,
            0,
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("guardrails"),
            "the error names the pane: {err:?}"
        );
        assert!(
            err.contains('4'),
            "and the smallest height that works: {err:?}"
        );
        // One content row is enough.
        assert!(
            Registry::panes(
                vec![spec("guardrails")],
                vec![pane(4, PaneWidth::Weight(1))],
                stacked(1),
                1,
                0,
            )
            .is_ok()
        );
    }

    #[test]
    fn ids_walk_the_registry_in_declaration_order() {
        // The combining hash and the runtime vec are both index-keyed
        // (I-45), so this order is a contract, not a convenience.
        let registry = Registry::panes(
            vec![spec("plan"), spec("git"), spec("guardrails")],
            vec![
                pane(7, PaneWidth::Weight(1)),
                pane(7, PaneWidth::Cells(20)),
                pane(7, PaneWidth::Weight(1)),
            ],
            LayoutNode::Column(vec![
                LayoutNode::Pane(SourceId(0)),
                LayoutNode::Row(vec![
                    LayoutNode::Pane(SourceId(1)),
                    LayoutNode::Pane(SourceId(2)),
                ]),
            ]),
            1,
            0,
        )
        .unwrap();
        assert_eq!(
            registry.ids().collect::<Vec<_>>(),
            vec![SourceId(0), SourceId(1), SourceId(2)]
        );
        assert_eq!(registry.spec(SourceId(2)).name, "guardrails");
        let geom = registry.geometry((80, 40));
        // The stacked pane owns the full width; the row splits it: the
        // fixed pane takes 20, the weighted one the rest minus the gap.
        assert_eq!(geom[0].cells, 80);
        assert_eq!(geom[1].cells, 20);
        assert_eq!(geom[2].cells, 59);
    }
}
