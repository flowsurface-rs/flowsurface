use super::{DragAction, HorizontalWheelZoomMode, LayerEffects, PointerMotion, UserInteraction};
use crate::widget::chart::kline::{
    KlineWidgetEvent, PANEL_SPLITTER_HEIGHT, PanelInteraction, chrome,
    composition::PanelScaleMode,
    layout::{LayoutHitZone, PanelLayoutTree},
    painting::RenderRequest,
    scene::{CursorTarget, Scene, viewport::PanelViewportView},
};
use data::chart::kline::HorizontalWorld;
use data::chart::kline::viewport::PanelYValueRange;
use iced::{Point, Vector, keyboard, mouse};
use iced_core::mouse::click;

const PIXELS_PER_SCROLL_LINE: f32 = 40.0;
const HORIZONTAL_DRAG_SCALE_DELTA_PER_PX: f32 = 0.3 / 9.0;
const Y_AXIS_DRAG_SCALE_DELTA_PER_PX: f32 = 0.1;

pub(crate) struct NavigationView<'a> {
    pub(crate) viewports: PanelViewportView<'a>,
    pub(crate) horizontal_world: HorizontalWorld,
}

impl NavigationView<'_> {
    pub(crate) fn wheel(
        &self,
        delta: mouse::ScrollDelta,
        zone: LayoutHitZone,
        position: Point,
        modifiers: keyboard::Modifiers,
        scene: Option<&Scene>,
    ) -> LayerEffects<KlineWidgetEvent> {
        let scroll_y = match delta {
            mouse::ScrollDelta::Lines { y, .. } => y,
            mouse::ScrollDelta::Pixels { y, .. } => y / PIXELS_PER_SCROLL_LINE,
        };
        if !scroll_y.is_finite() || scroll_y.abs() <= f32::EPSILON {
            return LayerEffects::default();
        }
        let Some(scene) = scene else {
            return LayerEffects::default();
        };
        match zone {
            LayoutHitZone::PanelPlot(_)
            | LayoutHitZone::PanelXAxis(_)
            | LayoutHitZone::BottomXAxis => {
                if matches!(zone, LayoutHitZone::PanelPlot(_))
                    && matches!(
                        scene.interaction.target,
                        CursorTarget::PanelControl { .. }
                            | CursorTarget::CornerControl(_)
                            | CursorTarget::Legend(_)
                    )
                {
                    return LayerEffects::default();
                }
                let zoom_mode = if matches!(
                    zone,
                    LayoutHitZone::PanelXAxis(_) | LayoutHitZone::BottomXAxis
                ) {
                    HorizontalWheelZoomMode::Cursor
                } else {
                    HorizontalWheelZoomMode::from_modifiers(modifiers)
                };
                let delta = scroll_y / 2.0;
                if delta.abs() <= f32::EPSILON {
                    return LayerEffects::default();
                }
                let right_edge = scene.x_axis_plot_width().max(1.0);
                let anchor_x =
                    self.horizontal_zoom_anchor(scene, position.x, right_edge, zoom_mode);
                let world = self.horizontal_world.zoom_around_screen(
                    scene.effective_right_x_unit(),
                    anchor_x,
                    right_edge,
                    delta,
                );
                self.horizontal_zoom_effects(world)
            }
            LayoutHitZone::YAxis(panel_index) => {
                let Some(viewport) = self.viewports.panel_y_zoom_range(scene, panel_index) else {
                    return LayerEffects::default();
                };
                self.y_zoom(
                    scene,
                    panel_index,
                    position.y,
                    viewport,
                    scroll_y.clamp(-6.0, 6.0),
                )
            }
            _ => LayerEffects::default(),
        }
    }

    pub(crate) fn axis_reset(
        &self,
        zone: LayoutHitZone,
        click_kind: Option<click::Kind>,
    ) -> Option<LayerEffects<KlineWidgetEvent>> {
        if click_kind != Some(click::Kind::Double) {
            return None;
        }
        let event = match zone {
            LayoutHitZone::YAxis(panel_index) => {
                let panel_id = self.viewports.coordinates.panel_id(panel_index)?;
                KlineWidgetEvent::PanelInteraction(PanelInteraction::YViewportReset { panel_id })
            }
            LayoutHitZone::PanelXAxis(_) | LayoutHitZone::BottomXAxis => {
                KlineWidgetEvent::XAxisDoubleClick
            }
            _ => return None,
        };
        let mut effects = LayerEffects {
            render_request: RenderRequest::All,
            ..LayerEffects::default()
        };
        effects.publish(event);
        Some(effects)
    }

    pub(crate) fn begin_axis_drag(
        &self,
        zone: LayoutHitZone,
        layout: &PanelLayoutTree,
        scene: Option<&Scene>,
        position: Point,
        interaction: &mut UserInteraction,
    ) -> Option<LayerEffects<KlineWidgetEvent>> {
        let mut effects = LayerEffects::captured(RenderRequest::None);
        let action = match zone {
            LayoutHitZone::Splitter(index) => DragAction::Split(index),
            LayoutHitZone::YAxis(panel_index) => {
                let initial_viewport = self.viewports.panel_y_zoom_range(scene?, panel_index)?;
                let coordinates = self.viewports.coordinates;
                let primary_panel_id = coordinates.composition.primary_panel_id();
                let is_primary_axis = coordinates.panel_id(panel_index) == primary_panel_id;
                let allows_primary_manual_zoom = primary_panel_id.is_some_and(|id| {
                    !matches!(
                        coordinates.composition.panel_effective_scale_mode(id),
                        Some(PanelScaleMode::PercentFromBase)
                    )
                });
                if is_primary_axis && allows_primary_manual_zoom {
                    effects.publish(KlineWidgetEvent::PrimaryAutoscaleDisabled);
                }
                let anchor_y = layout.panel(panel_index).map_or(position.y, |panel| {
                    layout.regions.plot.y + panel.plot.center().y
                });
                DragAction::YAxisScale {
                    panel_index,
                    anchor_y,
                    initial_viewport,
                }
            }
            LayoutHitZone::PanelXAxis(_) | LayoutHitZone::BottomXAxis => DragAction::XAxisScale {
                initial_world: self.horizontal_world,
                initial_right_unit: scene?.effective_right_x_unit(),
            },
            _ => return None,
        };
        interaction.begin_drag_candidate(position, action);
        Some(effects)
    }

    pub(crate) fn drag(
        &self,
        action: DragAction,
        motion: PointerMotion,
        delta: Vector,
        layout: &PanelLayoutTree,
        scene: Option<&Scene>,
    ) -> LayerEffects<KlineWidgetEvent> {
        match action {
            DragAction::XAxisScale {
                initial_world,
                initial_right_unit,
            } => {
                let total_delta = (motion.origin.x - motion.current.x).round()
                    * HORIZONTAL_DRAG_SCALE_DELTA_PER_PX;
                if total_delta.abs() > f32::EPSILON
                    && let Some(scene) = scene
                {
                    let right_edge = scene.x_axis_plot_width().max(1.0);
                    let anchor_x =
                        (motion.origin.x - scene.layout.regions.plot.x).clamp(0.0, right_edge);
                    let world = initial_world.zoom_around_screen(
                        initial_right_unit,
                        anchor_x,
                        right_edge,
                        total_delta,
                    );
                    self.horizontal_zoom_effects(world)
                } else {
                    LayerEffects::default()
                }
            }
            DragAction::Split(index) => {
                let Some(split) =
                    layout.split_ratio_from_cursor(motion.current.y, index, PANEL_SPLITTER_HEIGHT)
                else {
                    return LayerEffects::default();
                };
                let mut effects = LayerEffects::captured(RenderRequest::All);
                effects.publish(KlineWidgetEvent::PanelInteraction(
                    PanelInteraction::SplitChanged { index, split },
                ));
                effects
            }
            DragAction::YAxisScale {
                panel_index,
                anchor_y,
                initial_viewport,
            } => {
                let scale_delta =
                    (motion.origin.y - motion.current.y).round() * Y_AXIS_DRAG_SCALE_DELTA_PER_PX;
                if let Some(scene) = scene {
                    self.y_zoom(scene, panel_index, anchor_y, initial_viewport, scale_delta)
                } else {
                    LayerEffects::default()
                }
            }
            DragAction::Pan { panel_index, .. } => {
                self.pan(panel_index, motion.current, delta, scene)
            }
            _ => LayerEffects::default(),
        }
    }

    fn pan(
        &self,
        panel_index: usize,
        position: Point,
        delta: Vector,
        scene: Option<&Scene>,
    ) -> LayerEffects<KlineWidgetEvent> {
        let mut effects = LayerEffects::default();
        if scene.is_some_and(|scene| {
            chrome::hit_corner_control(&scene.chrome.corner_controls, position).is_some()
        }) {
            return effects;
        }
        let offset = self.horizontal_world.offset() - delta.x / self.horizontal_world.spacing();
        if (offset - self.horizontal_world.offset()).abs() > f32::EPSILON {
            effects.publish(KlineWidgetEvent::HorizontalOffsetChanged(offset));
            effects.render_request = RenderRequest::All;
        }
        if delta.y.abs() > f32::EPSILON
            && let Some(scene) = scene
            && let Some(panel_id) = self.viewports.coordinates.panel_id(panel_index)
            && let Some(range) =
                self.viewports
                    .panel_viewport_after_primary_y_pan(scene, panel_index, delta.y)
        {
            effects.publish(KlineWidgetEvent::PanelInteraction(
                PanelInteraction::YViewportChanged { panel_id, range },
            ));
            effects.render_request = RenderRequest::All;
        }
        effects
    }

    fn y_zoom(
        &self,
        scene: &Scene,
        panel_index: usize,
        anchor_y: f32,
        viewport: PanelYValueRange,
        delta: f32,
    ) -> LayerEffects<KlineWidgetEvent> {
        let Some(panel_id) = self.viewports.coordinates.panel_id(panel_index) else {
            return LayerEffects::default();
        };
        let Some(range) = self.viewports.panel_viewport_after_y_zoom(
            scene,
            panel_index,
            anchor_y,
            viewport,
            delta,
        ) else {
            return LayerEffects::default();
        };
        let mut effects = LayerEffects::captured(RenderRequest::All);
        effects.publish(KlineWidgetEvent::PanelInteraction(
            PanelInteraction::YViewportChanged { panel_id, range },
        ));
        effects
    }

    fn horizontal_zoom_effects(&self, world: HorizontalWorld) -> LayerEffects<KlineWidgetEvent> {
        if world == self.horizontal_world {
            return LayerEffects::default();
        }
        let mut effects = LayerEffects::captured(RenderRequest::All);
        effects.publish(KlineWidgetEvent::HorizontalWorldChanged(world));
        effects
    }

    fn horizontal_zoom_anchor(
        &self,
        scene: &Scene,
        cursor_x: f32,
        plot_width: f32,
        zoom_mode: HorizontalWheelZoomMode,
    ) -> f32 {
        let cursor_x = (cursor_x - scene.layout.regions.plot.x).clamp(0.0, plot_width);
        if matches!(zoom_mode, HorizontalWheelZoomMode::Cursor) {
            return cursor_x;
        }
        let latest_x = scene.map_x_plot(0);
        if latest_x.is_finite() && (0.0..=plot_width).contains(&latest_x) {
            latest_x
        } else {
            plot_width
        }
    }
}
