use egui::{Ui, WidgetText, Rect, Pos2, Vec2, Color32, Stroke, Align2, FontId, Sense, Id, Rounding};
use crate::{Plugin, AppCommand, TabInstance, Tab};
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

mod logic;
mod graph;

use self::graph::*;
use egui_node_graph2::*;

const GRID: f32 = 20.0;
const SUB_GRID: f32 = 10.0; // 允许对齐到点与点之间

#[derive(Serialize, Deserialize)]
struct ProjectData {
    pub title: String,
}

#[derive(Clone)]
pub struct CircuitCanvasTab {
    state: MyEditorState,
    user_state: MyGraphState,
    file_path: Option<PathBuf>,
    
    pan_offset: Vec2,
    zoom: f32,
    is_collapsed: bool,
    dragged_template: Option<MyNodeTemplate>,
    swipe_last_pos: Option<Pos2>,
    secondary_click_start: Option<Pos2>,
    is_swiping: bool,
    
    active_drag_source: Option<OutputId>, 
    dragged_node: Option<NodeId>,

    wires: Vec<Vec<Pos2>>,
    current_wire: Option<Vec<Pos2>>,
}

impl std::fmt::Debug for CircuitCanvasTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CircuitCanvasTab").finish()
    }
}

impl CircuitCanvasTab {
    pub fn new() -> Self {
        let mut state = MyEditorState::default();
        state.pan_zoom.zoom = 1.0;
        
        Self {
            state,
            user_state: MyGraphState::default(),
            file_path: None,
            pan_offset: Vec2::ZERO,
            zoom: 1.0,
            is_collapsed: false,
            dragged_template: None,
            swipe_last_pos: None,
            secondary_click_start: None,
            is_swiping: false,
            active_drag_source: None,
            dragged_node: None,
            wires: Vec::new(),
            current_wire: None,
        }
    }

    // 节点对齐 (粗网格)
    fn snap_node(pos: Pos2) -> Pos2 {
        Pos2::new(
            (pos.x / GRID).round() * GRID,
            (pos.y / GRID).round() * GRID,
        )
    }

    // 导线对齐 (细网格，支持点与点之间)
    fn snap_wire(pos: Pos2) -> Pos2 {
        Pos2::new(
            (pos.x / SUB_GRID).round() * SUB_GRID,
            (pos.y / SUB_GRID).round() * SUB_GRID,
        )
    }

    fn get_node_size(&self, node: &Node<MyNodeData>) -> Vec2 {
        let port_count = node.inputs.len().max(node.outputs.len());
        let width = 3.0 * GRID;
        let height = ((port_count as f32) + 1.0).max(3.0) * GRID;
        Vec2::new(width, height)
    }

    fn get_port_pos(&self, node_pos: Pos2, node_size: Vec2, port_index: usize, is_output: bool) -> Pos2 {
        let x = if is_output { node_pos.x + node_size.x } else { node_pos.x };
        // 1.5 * GRID 确保引脚位于半格位置，现在 snap_wire 可以对齐它了
        let y = node_pos.y + (1.5 + port_index as f32) * GRID;
        Pos2::new(x, y)
    }

    fn draw_polyline(&self, painter: &egui::Painter, from: Pos2, to: Pos2, color: Color32, canvas_rect: Rect) {
        let mut current = from;
        let target = to;
        let stroke = Stroke::new(2.0 * self.zoom, color);

        let mut dx = target.x - current.x;
        let mut dy = target.y - current.y;

        let mut limit = 0;
        while (dx.abs() > 0.1 || dy.abs() > 0.1) && limit < 100 {
            let next;
            if dx.abs() > 0.1 && dy.abs() > 0.1 {
                let step = dx.abs().min(dy.abs());
                next = Pos2::new(
                    current.x + step * dx.signum(),
                    current.y + step * dy.signum(),
                );
            } else if dx.abs() > 0.1 {
                next = Pos2::new(target.x, current.y);
            } else {
                next = Pos2::new(current.x, target.y);
            }

            let p1 = self.world_to_screen(current, canvas_rect);
            let p2 = self.world_to_screen(next, canvas_rect);
            painter.line_segment([p1, p2], stroke);
            
            current = next;
            dx = target.x - current.x;
            dy = target.y - current.y;
            limit += 1;
        }
    }

    fn world_to_screen(&self, world_pos: Pos2, canvas_rect: Rect) -> Pos2 { 
        canvas_rect.min + (world_pos.to_vec2() + self.pan_offset) * self.zoom 
    }
    
    fn screen_to_world(&self, screen_pos: Pos2, canvas_rect: Rect) -> Pos2 { 
        ((screen_pos - canvas_rect.min) / self.zoom - self.pan_offset).to_pos2() 
    }

    fn get_category_color(&self, gate_type: &logic::GateType) -> Color32 {
        use logic::GateType::*;
        match gate_type {
            And | Or | Xor => Color32::from_rgb(60, 60, 180),
            Not => Color32::from_rgb(180, 60, 60),
            Input => Color32::from_rgb(60, 180, 60),
            Output => Color32::from_rgb(180, 180, 60),
            _ => Color32::from_gray(100),
        }
    }
}

impl TabInstance for CircuitCanvasTab {
    fn title(&self) -> WidgetText { "Circuit Canvas".into() }

    fn ui(&mut self, ui: &mut Ui, _control: &mut Vec<AppCommand>) {
        egui::TopBottomPanel::top(ui.id().with("top_bar")).show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.selectable_label(!self.is_collapsed, "Gates").clicked() { self.is_collapsed = !self.is_collapsed; }
                ui.separator();
                ui.label(format!("{:.1}x | Grid Snap: {}px", self.zoom, SUB_GRID));
            });
        });

        egui::SidePanel::left(ui.id().with("toolbox")).show_animated_inside(ui, !self.is_collapsed, |ui| {
            ui.vertical(|ui| {
                for kind in MyNodeTemplate::And.all_kinds() {
                    let (rect, resp) = ui.allocate_at_least(Vec2::new(ui.available_width(), 30.0), Sense::click_and_drag());
                    ui.painter().rect_filled(rect, 2.0, Color32::from_gray(60));
                    ui.painter().text(rect.center(), Align2::CENTER_CENTER, format!("{:?}", kind), FontId::proportional(12.0), Color32::WHITE);
                    if resp.drag_started() { self.dragged_template = Some(kind); }
                }
            });
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let canvas_rect = ui.available_rect_before_wrap();
            let canvas_resp = ui.interact(canvas_rect, ui.id().with("canvas"), Sense::click_and_drag());
            let m_pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or(Pos2::ZERO);
            let world_m_pos = self.screen_to_world(m_pos, canvas_rect);
            let snapped_wire_m_pos = Self::snap_wire(world_m_pos);

            if canvas_resp.dragged_by(egui::PointerButton::Middle) { self.pan_offset += canvas_resp.drag_delta() / self.zoom; }
            let scroll = ui.input(|i| i.raw_scroll_delta.y);
            if scroll != 0.0 && canvas_resp.hovered() {
                let zoom_delta = if scroll > 0.0 { 1.1 } else { 0.9 };
                let old_zoom = self.zoom;
                self.zoom = (self.zoom * zoom_delta).clamp(0.2, 3.0);
                let world_anchor = ((m_pos - canvas_rect.min) / old_zoom - self.pan_offset).to_pos2();
                self.pan_offset = (m_pos - canvas_rect.min) / self.zoom - world_anchor.to_vec2();
            }

            let painter = ui.painter_at(canvas_rect);
            painter.rect_filled(canvas_rect, 0.0, Color32::from_gray(30));

            let grid_spacing = GRID * self.zoom;
            let off_x = (self.pan_offset.x * self.zoom) % grid_spacing;
            let off_y = (self.pan_offset.y * self.zoom) % grid_spacing;
            for x in std::iter::successors(Some(off_x), |&x| Some(x + grid_spacing)).take_while(|&x| x < canvas_rect.width()) {
                for y in std::iter::successors(Some(off_y), |&y| Some(y + grid_spacing)).take_while(|&y| y < canvas_rect.height()) {
                    painter.circle_filled(canvas_rect.min + Vec2::new(x, y), 1.0, Color32::from_gray(60));
                }
            }

            let is_secondary_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Secondary));
            if is_secondary_down {
                if let Some(last) = self.swipe_last_pos {
                    let swipe_rect = Rect::from_two_pos(last, world_m_pos).expand(2.0);
                    let mut to_remove = Vec::new();
                    for (id, pos) in self.state.node_positions.iter() {
                        let size = self.get_node_size(&self.state.graph.nodes[id]);
                        if Rect::from_min_size(*pos, size).intersects(swipe_rect) { to_remove.push(id); }
                    }
                    for id in to_remove { self.state.graph.remove_node(id); self.state.node_positions.remove(id); }
                    self.wires.retain(|w| !w.windows(2).any(|seg| Rect::from_two_pos(seg[0], seg[1]).expand(2.0).intersects(swipe_rect)));
                }
                self.swipe_last_pos = Some(world_m_pos);
            } else { self.swipe_last_pos = None; }

            for (input_id, output_ids) in self.state.graph.connections.iter() {
                for &output_id in output_ids {
                    let out_node = self.state.graph.outputs[output_id].node;
                    let in_node = self.state.graph.inputs[input_id].node;
                    if let (Some(p1_w), Some(p2_w)) = (self.state.node_positions.get(out_node), self.state.node_positions.get(in_node)) {
                        let s1 = self.get_node_size(&self.state.graph.nodes[out_node]);
                        let s2 = self.get_node_size(&self.state.graph.nodes[in_node]);
                        let out_idx = self.state.graph.nodes[out_node].outputs.iter().position(|(_, id)| *id == output_id).unwrap_or(0);
                        let in_idx = self.state.graph.nodes[in_node].inputs.iter().position(|(_, id)| *id == input_id).unwrap_or(0);
                        let p1 = self.get_port_pos(*p1_w, s1, out_idx, true);
                        let p2 = self.get_port_pos(*p2_w, s2, in_idx, false);
                        self.draw_polyline(&painter, p1, p2, Color32::GREEN, canvas_rect);
                    }
                }
            }

            for wire in &self.wires {
                for seg in wire.windows(2) {
                    self.draw_polyline(&painter, seg[0], seg[1], Color32::WHITE, canvas_rect);
                }
            }

            let mut hover_input = None;
            let ids: Vec<NodeId> = self.state.graph.nodes.keys().collect();
            for id in ids {
                let node = &self.state.graph.nodes[id];
                let pos = self.state.node_positions[id];
                let node_size = self.get_node_size(node);
                let s_pos = self.world_to_screen(pos, canvas_rect);
                let s_size = node_size * self.zoom;
                let r = Rect::from_min_size(s_pos, s_size);

                painter.rect_filled(r, 0.0, Color32::from_gray(45));
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_gray(80)));
                let h_r = Rect::from_min_size(s_pos, Vec2::new(s_size.x, GRID * self.zoom));
                painter.rect_filled(h_r, Rounding::ZERO, self.get_category_color(&node.user_data.gate_type));
                painter.text(h_r.center(), Align2::CENTER_CENTER, format!("{:?}", node.user_data.gate_type), FontId::proportional(10.0 * self.zoom), Color32::WHITE);

                let resp = ui.interact(r, Id::new("node").with(id), Sense::drag());
                if resp.dragged() {
                    let new_pos = self.screen_to_world(resp.interact_pointer_pos().unwrap_or(m_pos), canvas_rect) - node_size / 2.0;
                    self.state.node_positions.insert(id, Self::snap_node(new_pos));
                }

                for (i, &(_, in_id)) in node.inputs.iter().enumerate() {
                    let p = self.world_to_screen(self.get_port_pos(pos, node_size, i, false), canvas_rect);
                    painter.circle_filled(p, 3.0 * self.zoom, Color32::LIGHT_GRAY);
                    if (m_pos - p).length() < 10.0 { hover_input = Some(in_id); }
                }
                for (i, &(_, out_id)) in node.outputs.iter().enumerate() {
                    let p = self.world_to_screen(self.get_port_pos(pos, node_size, i, true), canvas_rect);
                    painter.circle_filled(p, 3.0 * self.zoom, Color32::LIGHT_GRAY);
                    let p_resp = ui.interact(Rect::from_center_size(p, Vec2::splat(15.0)), Id::new("p_out").with(out_id), Sense::drag());
                    if p_resp.drag_started() { self.active_drag_source = Some(out_id); }
                }
            }

            if ui.input(|i| i.pointer.primary_pressed()) && !ui.ctx().is_using_pointer() {
                self.current_wire = Some(vec![snapped_wire_m_pos]);
            }
            if let Some(wire) = &mut self.current_wire {
                let start = wire[0];
                self.draw_polyline(&painter, start, snapped_wire_m_pos, Color32::YELLOW, canvas_rect);
                if ui.input(|i| i.pointer.primary_released()) {
                    if (snapped_wire_m_pos - start).length() > 5.0 { self.wires.push(vec![start, snapped_wire_m_pos]); }
                    self.current_wire = None;
                }
            }

            if let Some(out_id) = self.active_drag_source {
                let out_node = self.state.graph.outputs[out_id].node;
                let out_pos = self.state.node_positions[out_node];
                let out_idx = self.state.graph.nodes[out_node].outputs.iter().position(|(_, id)| *id == out_id).unwrap_or(0);
                let p1 = self.get_port_pos(out_pos, self.get_node_size(&self.state.graph.nodes[out_node]), out_idx, true);
                self.draw_polyline(&painter, p1, snapped_wire_m_pos, Color32::YELLOW, canvas_rect);
                if ui.input(|i| i.pointer.any_released()) {
                    if let Some(in_id) = hover_input { self.state.graph.add_connection(out_id, in_id, 0); }
                    self.active_drag_source = None;
                }
            }

            if let Some(t) = self.dragged_template {
                painter.text(m_pos, Align2::CENTER_CENTER, format!("{:?}", t), FontId::proportional(12.0), Color32::WHITE);
                if ui.input(|i| i.pointer.any_released()) {
                    let id = self.state.graph.add_node(format!("{:?}", t), t.user_data(&mut self.user_state), |graph, node_id| { t.build_node(graph, &mut self.user_state, node_id); });
                    self.state.node_positions.insert(id, snapped_wire_m_pos);
                    self.dragged_template = None;
                }
            }
        });
    }
    fn box_clone(&self) -> Box<dyn TabInstance> { Box::new(self.clone()) }
}

pub struct TuringNextPlugin;
impl Plugin for TuringNextPlugin {
    fn name(&self) -> &str { crate::plugins::PLUGIN_NAME_TURING_NEXT }
    fn on_tab_menu(&mut self, ui: &mut Ui, control: &mut Vec<AppCommand>) {
        if ui.button("New Circuit Canvas").clicked() {
            control.push(AppCommand::OpenTab(Tab::new(Box::new(CircuitCanvasTab::new()))));
            ui.close_menu();
        }
    }
}
pub fn create() -> TuringNextPlugin { TuringNextPlugin }
