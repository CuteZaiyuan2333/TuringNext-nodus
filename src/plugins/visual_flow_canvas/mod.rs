use egui::{Ui, WidgetText, Rect, Pos2, Vec2, Color32, Stroke, Align2, FontId, Sense, Id, Rounding};
use crate::{Plugin, AppCommand, TabInstance, Tab, NotificationLevel};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use serde::{Serialize, Deserialize};

mod compiler;

// ----------------------------------------------------------------------------
// Core Types
// ----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub usize);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DataType {
    Flow, Any, String, Number, Bool,
}

impl DataType {
    pub fn color(&self) -> Color32 {
        match self {
            DataType::Flow => Color32::WHITE,
            DataType::Any => Color32::from_gray(200),
            DataType::String => Color32::from_rgb(255, 200, 100), 
            DataType::Number => Color32::from_rgb(100, 200, 255), 
            DataType::Bool => Color32::from_rgb(255, 100, 100),   
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Port {
    pub name: String,
    pub data_type: DataType,
}

mod serde_egui {
    use super::*;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct SerializableVec2 {
        x: f32,
        y: f32,
    }

    pub mod pos2 {
        use super::*;
        pub fn serialize<S>(pos: &Pos2, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let s = SerializableVec2 { x: pos.x, y: pos.y };
            s.serialize(serializer)
        }

        pub fn deserialize<'de, D>(deserializer: D) -> Result<Pos2, D::Error>
        where
            D: Deserializer<'de>,
        {
            let s = SerializableVec2::deserialize(deserializer)?;
            Ok(Pos2::new(s.x, s.y))
        }
    }

    pub mod vec2 {
        use super::*;
        pub fn serialize<S>(v: &Vec2, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let s = SerializableVec2 { x: v.x, y: v.y };
            s.serialize(serializer)
        }

        pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec2, D::Error>
        where
            D: Deserializer<'de>,
        {
            let s = SerializableVec2::deserialize(deserializer)?;
            Ok(Vec2::new(s.x, s.y))
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeTemplate {
    pub name: String,
    pub category: String,
    pub widget_type: Option<String>, 
    pub inputs: Vec<Port>,
    pub outputs: Vec<Port>,
    pub rhai_fn_name: String,
    pub rhai_body: String, 
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeInstance {
    pub id: NodeId,
    pub template_name: String,
    #[serde(with = "serde_egui::pos2")]
    pub position: Pos2, 
    #[serde(with = "serde_egui::vec2")]
    pub size: Vec2,
    #[serde(default)]
    pub data: String, 
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Connection {
    pub from_node: NodeId,
    pub from_port: usize, 
    pub to_node: NodeId,
    pub to_port: usize,   
}

// ----------------------------------------------------------------------------
// Async I/O Result types
// ----------------------------------------------------------------------------
enum IoResult {
    LoadedBlueprint { nodes: HashMap<NodeId, NodeInstance>, connections: Vec<Connection>, path: PathBuf },
    #[allow(dead_code)]
    LoadedTemplates(Vec<NodeTemplate>),
    Saved(PathBuf),
    Error(String),
}

// ----------------------------------------------------------------------------
// Tab Instance
// ----------------------------------------------------------------------------

#[derive(Clone)]
pub struct VisualFlowCanvasTab {
    nodes: HashMap<NodeId, NodeInstance>,
    connections: Vec<Connection>,
    templates: Vec<NodeTemplate>,
    
    next_node_id: usize,
    pan_offset: Vec2,
    zoom: f32,
    
    dragged_node: Option<NodeId>,
    selected_nodes: HashSet<NodeId>,
    last_selected_node: Option<NodeId>, 
    active_drag_source: Option<(NodeId, usize)>, 
    selection_start: Option<Pos2>, 
    swipe_last_pos: Option<Pos2>,  
    secondary_click_start: Option<Pos2>,
    secondary_started_on_node: bool,
    is_swiping: bool,
    context_menu_pos: Option<Pos2>,
    is_collapsed: bool,
    dragged_template: Option<NodeTemplate>,

    io_rx: Arc<Mutex<Receiver<IoResult>>>,
    io_tx: Sender<IoResult>,

    undo_stack: Vec<HistoryState>,
    redo_stack: Vec<HistoryState>,
}

#[derive(Clone, PartialEq)]
struct HistoryState {
    nodes: HashMap<NodeId, NodeInstance>,
    connections: Vec<Connection>,
    next_node_id: usize,
}

impl std::fmt::Debug for VisualFlowCanvasTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VisualFlowCanvasTab")
            .field("nodes_count", &self.nodes.len())
            .field("connections_count", &self.connections.len())
            .finish()
    }
}

impl VisualFlowCanvasTab {
    fn push_undo(&mut self) {
        let state = HistoryState {
            nodes: self.nodes.clone(),
            connections: self.connections.clone(),
            next_node_id: self.next_node_id,
        };
        // 只有当状态真正改变时才推送（简单检查）
        if let Some(last) = self.undo_stack.last() {
            if last == &state {
                return;
            }
        }
        self.undo_stack.push(state);
        self.redo_stack.clear();
        if self.undo_stack.len() > 50 { self.undo_stack.remove(0); }
    }

    fn undo(&mut self) {
        if let Some(current) = self.undo_stack.pop() {
            let redo_state = HistoryState {
                nodes: self.nodes.clone(),
                connections: self.connections.clone(),
                next_node_id: self.next_node_id,
            };
            self.redo_stack.push(redo_state);
            self.nodes = current.nodes;
            self.connections = current.connections;
            self.next_node_id = current.next_node_id;
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo_stack.pop() {
            let undo_state = HistoryState {
                nodes: self.nodes.clone(),
                connections: self.connections.clone(),
                next_node_id: self.next_node_id,
            };
            self.undo_stack.push(undo_state);
            self.nodes = next.nodes;
            self.connections = next.connections;
            self.next_node_id = next.next_node_id;
        }
    }

    pub fn new(templates: Vec<NodeTemplate>) -> Self {
        let (tx, rx) = channel();
        let mut tab = Self {
            nodes: HashMap::new(),
            connections: Vec::new(),
            templates,
            next_node_id: 0, pan_offset: Vec2::ZERO, zoom: 1.0, dragged_node: None,
            selected_nodes: HashSet::new(), last_selected_node: None,
            active_drag_source: None, selection_start: None, swipe_last_pos: None,
            secondary_click_start: None, secondary_started_on_node: false, is_swiping: false, context_menu_pos: None,
            is_collapsed: false, dragged_template: None,
            io_rx: Arc::new(Mutex::new(rx)), io_tx: tx,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        };
        if tab.templates.iter().any(|t| t.name == "Start") {
             tab.add_node("Start", Pos2::new(50.0, 50.0));
        }
        tab
    }

    fn process_async_io(&mut self, control: &mut Vec<AppCommand>) {
        if let Ok(rx) = self.io_rx.lock() {
            while let Ok(result) = rx.try_recv() {
                match result {
                    IoResult::LoadedBlueprint { nodes, connections, path } => {
                        self.nodes = nodes;
                        self.connections = connections;
                        self.next_node_id = self.nodes.keys().map(|id| id.0).max().unwrap_or(0) + 1;
                        control.push(AppCommand::Notify { 
                            message: format!("Loaded: {:?}", path.file_name().unwrap_or_default()), 
                            level: NotificationLevel::Success 
                        });
                    }
                                    IoResult::LoadedTemplates(new_templates) => {
                                        self.templates.extend(new_templates);
                                    }
                                    IoResult::Saved(path) => {
                                        control.push(AppCommand::Notify { 
                                            message: format!("Saved to: {:?}", path.file_name().unwrap_or_default()), 
                                            level: NotificationLevel::Success 
                                        });
                                    }
                                    IoResult::Error(e) => {                        control.push(AppCommand::Notify { message: e, level: NotificationLevel::Error });
                    }
                }
            }
        }
    }

    fn add_node(&mut self, template_name: &str, pos: Pos2) -> NodeId {
        self.push_undo();
        let id = NodeId(self.next_node_id); self.next_node_id += 1;
        let t = self.templates.iter().find(|t| t.name == template_name).unwrap();
        let max_ports = t.inputs.len().max(t.outputs.len());
        let widget_h = if t.widget_type.is_some() { 25.0 } else { 0.0 };
        let height = 35.0 + widget_h + (max_ports as f32 * 22.0).max(22.0);
        self.nodes.insert(id, NodeInstance { id, template_name: template_name.to_string(), position: pos, size: Vec2::new(150.0, height), data: String::new() });
        id
    }

    fn world_to_screen(&self, world_pos: Pos2, canvas_rect: Rect) -> Pos2 { canvas_rect.min + (world_pos.to_vec2() + self.pan_offset) * self.zoom }
    fn screen_to_world(&self, screen_pos: Pos2, canvas_rect: Rect) -> Pos2 { ((screen_pos - canvas_rect.min) / self.zoom - self.pan_offset).to_pos2() }
    
    fn get_category_color(&self, category: &str) -> Color32 {
        if category.contains("Flow") { Color32::from_rgb(180, 60, 60) }
        else if category.contains("IO") { Color32::from_rgb(60, 180, 60) }
        else if category.contains("Math") { Color32::from_rgb(60, 60, 180) }
        else { Color32::from_gray(120) }
    }

    fn get_port_world_pos(&self, node_id: NodeId, port_index: usize, is_output: bool) -> Pos2 {
        if let Some(node) = self.nodes.get(&node_id) {
            let mut start_y = node.position.y + 36.0;
            if let Some(t) = self.templates.iter().find(|t| t.name == node.template_name) {
                if t.widget_type.is_some() { start_y += 25.0; }
            }
            let x = if is_output { node.position.x + node.size.x } else { node.position.x };
            Pos2::new(x, start_y + port_index as f32 * 22.0)
        } else { Pos2::ZERO }
    }

    fn draw_port_shape(&self, painter: &egui::Painter, center: Pos2, dtype: &DataType, size: f32) {
        match dtype {
            DataType::Flow => {
                let h = size * 0.9; let w = size * 0.8;
                painter.add(egui::Shape::convex_polygon(vec![center + Vec2::new(h, 0.0), center + Vec2::new(-h, -w), center + Vec2::new(-h, w)], Color32::WHITE, Stroke::new(1.0, Color32::from_gray(50))));
            },
            _ => { painter.circle_filled(center, size * 0.8, dtype.color()); }
        }
    }

    fn draw_node_template_preview(&self, ui: &mut Ui, template: &NodeTemplate) -> egui::Response {
        let row_count = template.inputs.len().max(template.outputs.len()).max(1);
        let row_h = 15.0;
        let preview_height = 25.0 + (row_count as f32 * row_h);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), preview_height + 10.0), Sense::click_and_drag());
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let n_rect = Rect::from_center_size(rect.center(), Vec2::new(rect.width() - 20.0, preview_height));
            painter.rect_filled(n_rect, 4.0, Color32::from_gray(40));
            painter.rect_stroke(n_rect, 4.0, Stroke::new(1.0, Color32::from_gray(70)));
            let h_rect = Rect::from_min_size(n_rect.min, Vec2::new(n_rect.width(), 14.0));
            painter.rect_filled(h_rect, Rounding { nw: 4.0, ne: 4.0, sw: 0.0, se: 0.0 }, self.get_category_color(&template.category));
            painter.text(h_rect.center(), Align2::CENTER_CENTER, &template.name, FontId::proportional(9.0), Color32::WHITE);
            let row_y = n_rect.min.y + 20.0;
            let f_id = FontId::proportional(8.0);
            for (i, port) in template.inputs.iter().enumerate() {
                let py = row_y + i as f32 * row_h;
                self.draw_port_shape(painter, Pos2::new(n_rect.left(), py), &port.data_type, 3.0);
                if !port.name.is_empty() { painter.text(Pos2::new(n_rect.left() + 6.0, py), Align2::LEFT_CENTER, &port.name, f_id.clone(), Color32::WHITE); }
            }
            for (i, port) in template.outputs.iter().enumerate() {
                let py = row_y + i as f32 * row_h;
                self.draw_port_shape(painter, Pos2::new(n_rect.right(), py), &port.data_type, 3.0);
                if !port.name.is_empty() { painter.text(Pos2::new(n_rect.right() - 6.0, py), Align2::RIGHT_CENTER, &port.name, f_id.clone(), Color32::WHITE); }
            }
        }
        response
    }

    fn draw_connection(&self, painter: &egui::Painter, from: Pos2, to: Pos2, color: Color32) {
        let cp1 = from + Vec2::new(50.0 * self.zoom, 0.0);
        let cp2 = to + Vec2::new(-50.0 * self.zoom, 0.0);
        painter.add(egui::Shape::CubicBezier(egui::epaint::CubicBezierShape {
            points: [from, cp1, cp2, to], closed: false, fill: Color32::TRANSPARENT, stroke: Stroke::new(2.0 * self.zoom, color).into(),
        }));
    }
}

impl TabInstance for VisualFlowCanvasTab {
    fn title(&self) -> WidgetText { "VisualFlowCanvas".into() }

    fn ui(&mut self, ui: &mut Ui, control: &mut Vec<AppCommand>) {
        self.process_async_io(control);

        if ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Z)) { self.undo(); }
        if ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Y)) { self.redo(); }

        egui::TopBottomPanel::top(ui.id().with("top_bar")).show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.selectable_label(!self.is_collapsed, "Toolbox").clicked() { self.is_collapsed = !self.is_collapsed; }
                ui.separator();
                ui.label(format!("{:.1}x | Nodes: {}", self.zoom, self.nodes.len()));
                
                ui.separator();
                if ui.button("Save").clicked() {
                    let nodes = self.nodes.clone();
                    let connections = self.connections.clone();
                    let templates = self.templates.clone();
                    let tx = self.io_tx.clone();
                    std::thread::spawn(move || {
                        if let Some(path) = rfd::FileDialog::new().add_filter("Blueprint", &["vfrh"]).save_file() {
                            let mut compiler = compiler::FlowCompiler::new(&nodes, &connections, &templates);
                            match compiler.compile() {
                                Ok(code) => {
                                    if let Err(e) = std::fs::write(&path, code) {
                                        let _ = tx.send(IoResult::Error(format!("File Write Error: {}", e)));
                                    } else {
                                        let _ = tx.send(IoResult::Saved(path));
                                    }
                                }
                                Err(e) => {
                                    let _ = tx.send(IoResult::Error(format!("Compile Error: {}", e)));
                                }
                            }
                        }
                    });
                }

                if ui.button("Open").clicked() {
                    let tx = self.io_tx.clone();
                    let templates = self.templates.clone();
                    std::thread::spawn(move || {
                        if let Some(path) = rfd::FileDialog::new().add_filter("Blueprint", &["vfrh"]).pick_file() {
                            if let Ok(content) = std::fs::read_to_string(&path) {
                                match compiler::FlowLoader::parse(&content, &templates) {
                                    Ok((nodes, connections)) => { let _ = tx.send(IoResult::LoadedBlueprint { nodes, connections, path }); }
                                    Err(e) => { let _ = tx.send(IoResult::Error(format!("Parse Error: {}", e))); }
                                }
                            }
                        }
                    });
                }

                if ui.button("Compile").clicked() {
                    let mut compiler = compiler::FlowCompiler::new(&self.nodes, &self.connections, &self.templates);
                    if let Ok(code) = compiler.compile() {
                        println!("--- Generated Rhai Code ---\n{}\n---------------------------", code);
                        control.push(AppCommand::Notify { message: "Code generated!".into(), level: NotificationLevel::Info });
                    }
                }
            });
        });

        let toolbox_id = ui.id().with("toolbox");
        egui::SidePanel::left(toolbox_id).resizable(true).default_width(180.0).show_animated_inside(ui, !self.is_collapsed, |ui| {
            ui.vertical(|ui| {
                ui.heading("Toolbox"); ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut categories: HashMap<String, Vec<NodeTemplate>> = HashMap::new();
                    for t in &self.templates { categories.entry(t.category.clone()).or_default().push(t.clone()); }
                    let mut sorted_cat: Vec<_> = categories.keys().cloned().collect(); sorted_cat.sort();
                    for cat in sorted_cat {
                        ui.collapsing(&cat, |ui| {
                            for template in &categories[&cat] {
                                if self.draw_node_template_preview(ui, template).drag_started() { self.dragged_template = Some(template.clone()); }
                                ui.add_space(4.0);
                            }
                        });
                    }
                });
            });
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let canvas_rect = ui.available_rect_before_wrap();
            let canvas_resp = ui.interact(canvas_rect, ui.id().with("canvas"), Sense::click_and_drag());
            let m_pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or(Pos2::ZERO);
            let world_m_pos = self.screen_to_world(m_pos, canvas_rect);

            let scroll = ui.input(|i| i.raw_scroll_delta.y);
            if scroll != 0.0 && canvas_resp.hovered() {
                let zoom_delta = if scroll > 0.0 { 1.1 } else { 0.9 };
                let old_zoom = self.zoom;
                self.zoom = (self.zoom * zoom_delta).clamp(0.2, 3.0);
                let world_anchor = ((m_pos - canvas_rect.min) / old_zoom - self.pan_offset).to_pos2();
                self.pan_offset = (m_pos - canvas_rect.min) / self.zoom - world_anchor.to_vec2();
            }
            if canvas_resp.dragged_by(egui::PointerButton::Middle) { self.pan_offset += canvas_resp.drag_delta() / self.zoom; }

            let mut painter = ui.painter_at(canvas_rect);
            painter.set_clip_rect(canvas_rect);
            painter.rect_filled(canvas_rect, 0.0, Color32::from_gray(30));
            let grid_spacing = 20.0 * self.zoom;
            let off_x = (self.pan_offset.x * self.zoom) % grid_spacing;
            let off_y = (self.pan_offset.y * self.zoom) % grid_spacing;
            for x in std::iter::successors(Some(off_x), |&x| Some(x + grid_spacing)).take_while(|&x| x < canvas_rect.width()) {
                for y in std::iter::successors(Some(off_y), |&y| Some(y + grid_spacing)).take_while(|&y| y < canvas_rect.height()) {
                    painter.circle_filled(canvas_rect.min + Vec2::new(x, y), 1.0 * self.zoom.max(0.5), Color32::from_gray(60));
                }
            }

            let is_secondary_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Secondary));
            let is_secondary_pressed = ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Secondary));
            let is_secondary_released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Secondary));

            if is_secondary_pressed && canvas_rect.contains(m_pos) {
                self.secondary_click_start = Some(m_pos);
                self.secondary_started_on_node = self.nodes.values().any(|n| {
                    let sn_pos = self.world_to_screen(n.position, canvas_rect);
                    Rect::from_min_size(sn_pos, n.size * self.zoom).contains(m_pos)
                });
                self.is_swiping = false; self.context_menu_pos = None;
            }
            if is_secondary_down {
                if let Some(start) = self.secondary_click_start {
                    if (m_pos - start).length() > 10.0 && !self.secondary_started_on_node { self.is_swiping = true; }
                }
                if self.is_swiping {
                    if let Some(last) = self.swipe_last_pos {
                        let swipe_rect = Rect::from_two_pos(last, world_m_pos).expand(2.0);
                        let mut changed = false;
                        let old_node_count = self.nodes.len();
                        self.nodes.retain(|_, node| !swipe_rect.intersects(Rect::from_min_size(node.position, node.size)));
                        if self.nodes.len() != old_node_count { changed = true; }

                        let mut to_remove = Vec::new();
                        for (i, conn) in self.connections.iter().enumerate() {
                            if !self.nodes.contains_key(&conn.from_node) || !self.nodes.contains_key(&conn.to_node) { to_remove.push(i); continue; }
                            let p1 = self.get_port_world_pos(conn.from_node, conn.from_port, true);
                            let p2 = self.get_port_world_pos(conn.to_node, conn.to_port, false);
                            if Rect::from_two_pos(p1, p2).intersects(swipe_rect) { to_remove.push(i); }
                        }
                        if !to_remove.is_empty() { changed = true; }
                        if changed { self.push_undo(); }

                        for &i in to_remove.iter().rev() { self.connections.remove(i); }
                    }
                    self.swipe_last_pos = Some(world_m_pos);
                }
            }
            if is_secondary_released {
                if !self.is_swiping {
                    if let Some(start) = self.secondary_click_start { if (m_pos - start).length() < 5.0 { self.context_menu_pos = Some(m_pos); } }
                }
                self.swipe_last_pos = None; self.secondary_click_start = None; self.is_swiping = false;
            }

            for conn in &self.connections {
                let p1 = self.world_to_screen(self.get_port_world_pos(conn.from_node, conn.from_port, true), canvas_rect);
                let p2 = self.world_to_screen(self.get_port_world_pos(conn.to_node, conn.to_port, false), canvas_rect);
                
                // 性能优化：连线裁剪
                if !canvas_rect.intersects(Rect::from_two_pos(p1, p2).expand(20.0)) { continue; }

                let mut color = Color32::WHITE;
                if let Some(node) = self.nodes.get(&conn.from_node) {
                     if let Some(tmpl) = self.templates.iter().find(|t| t.name == node.template_name) {
                         if let Some(port) = tmpl.outputs.get(conn.from_port) { color = port.data_type.color(); }
                     }
                }
                self.draw_connection(&painter, p1, p2, color);
            }

            let mut node_to_drag = None;
            let mut node_to_select = None;
            let mut drag_delta = Vec2::ZERO;
            let mut hover_port = None;
            let mut start_drag_port = None;

            let mut sorted_ids: Vec<NodeId> = self.nodes.keys().cloned().collect();
            sorted_ids.sort_by(|a, b| if Some(*a) == self.last_selected_node { std::cmp::Ordering::Greater } else if Some(*b) == self.last_selected_node { std::cmp::Ordering::Less } else { std::cmp::Ordering::Equal });

            for id in sorted_ids {
                let (template, position, size) = {
                    let node = &self.nodes[&id];
                    let t = self.templates.iter().find(|t| t.name == node.template_name).unwrap();
                    (t.clone(), node.position, node.size)
                };
                let screen_pos = self.world_to_screen(position, canvas_rect);
                let screen_size = size * self.zoom;
                let node_rect = Rect::from_min_size(screen_pos, screen_size);
                if !canvas_rect.intersects(node_rect) { continue; }

                if self.selected_nodes.contains(&id) { painter.rect_stroke(node_rect.expand(2.0), 5.0 * self.zoom, Stroke::new(2.0 * self.zoom, Color32::from_rgb(200, 180, 0))); }
                painter.rect_filled(node_rect, 5.0 * self.zoom, Color32::from_gray(45));
                painter.rect_stroke(node_rect, 5.0 * self.zoom, Stroke::new(1.0, Color32::from_gray(80)));

                // 1. 先处理节点背景的交互 (拖拽/点击)
                let node_resp = ui.interact(node_rect, Id::new(id.0), Sense::drag());
                if node_resp.clicked() { node_to_select = Some(id); }
                if node_resp.drag_started() { node_to_drag = Some(id); node_to_select = Some(id); }
                if node_resp.dragged() { drag_delta = node_resp.drag_delta(); }
                if node_resp.drag_stopped() { self.push_undo(); }
                
                let h_rect = Rect::from_min_size(screen_pos, Vec2::new(screen_size.x, 24.0 * self.zoom));
                painter.rect_filled(h_rect, Rounding { nw: 5.0 * self.zoom, ne: 5.0 * self.zoom, sw: 0.0, se: 0.0 }, self.get_category_color(&template.category));
                painter.text(h_rect.center(), Align2::CENTER_CENTER, &template.name, FontId::proportional(13.0 * self.zoom), Color32::WHITE);

                let mut start_y_offset = 36.0;
                // 2. 后处理 Widget 交互 (在背景之上，可以覆盖背景的输入)
                let mut node_changed = false;
                if let Some(w_type) = &template.widget_type {
                    let w_rect = Rect::from_min_size(screen_pos + Vec2::new(5.0, 29.0) * self.zoom, Vec2::new(screen_size.x - 10.0 * self.zoom, 20.0 * self.zoom));
                    if let Some(node) = self.nodes.get_mut(&id) {
                        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(w_rect), |ui| {
                            match w_type.as_str() {
                                "string_input" | "number_input" => {
                                    if ui.add(egui::TextEdit::singleline(&mut node.data)
                                        .id_source(format!("edit_{}", id.0))
                                        .desired_width(f32::INFINITY)).changed() { node_changed = true; }
                                }
                                "checkbox" => {
                                    let mut checked = node.data == "true";
                                    if ui.checkbox(&mut checked, "").changed() {
                                        node.data = checked.to_string();
                                        node_changed = true;
                                    }
                                }
                                "dropdown" => {
                                    // 预先提取数据避免借用冲突
                                    let node_data_clone = node.data.clone();
                                    let parts: Vec<&str> = node_data_clone.split('|').collect();
                                    let current = parts.get(0).unwrap_or(&"").to_string();
                                    let options_str = parts.get(1).unwrap_or(&"").to_string();
                                    
                                    egui::ComboBox::from_id_salt(id.0)
                                        .selected_text(&current)
                                        .show_ui(ui, |ui| {
                                            for opt in options_str.split(',') {
                                                let target_data = format!("{}|{}", opt, options_str);
                                                if ui.selectable_value(&mut node.data, target_data, opt).changed() {
                                                    node_changed = true;
                                                }
                                            }
                                        });
                                }
                                _ => {}
                            }
                        });
                    }
                    start_y_offset += 25.0;
                }
                if node_changed { self.push_undo(); }

                let row_h = 22.0 * self.zoom;
                let start_y = screen_pos.y + start_y_offset * self.zoom;
                let mut draw_p = |p_idx: usize, is_output: bool, port: &Port, align: Align2, x: f32| {
                    let p_pos = Pos2::new(x, start_y + p_idx as f32 * row_h);
                    self.draw_port_shape(&painter, p_pos, &port.data_type, 5.0 * self.zoom);
                    if !port.name.is_empty() {
                        let off = if align == Align2::LEFT_CENTER { 12.0 } else { -12.0 } * self.zoom;
                        painter.text(p_pos + Vec2::new(off, 0.0), align, &port.name, FontId::proportional(11.0 * self.zoom), Color32::LIGHT_GRAY);
                    }
                    let p_resp = ui.interact(Rect::from_center_size(p_pos, Vec2::splat(20.0 * self.zoom)), ui.id().with(p_idx).with(is_output).with(id), Sense::drag());
                    if p_resp.hovered() { hover_port = Some((id, p_idx, is_output)); }
                    if p_resp.drag_started() && is_output { start_drag_port = Some((id, p_idx)); }
                };
                for (i, p) in template.inputs.iter().enumerate() { draw_p(i, false, p, Align2::LEFT_CENTER, node_rect.left()); }
                for (i, p) in template.outputs.iter().enumerate() { draw_p(i, true, p, Align2::RIGHT_CENTER, node_rect.right()); }
            }

            if let Some(p) = start_drag_port { self.active_drag_source = Some(p); self.dragged_node = None; }
            if let Some(id) = node_to_select {
                if ui.input(|i| i.modifiers.ctrl) { if self.selected_nodes.contains(&id) { self.selected_nodes.remove(&id); } else { self.selected_nodes.insert(id); } } 
                else { if !self.selected_nodes.contains(&id) { self.selected_nodes.clear(); self.selected_nodes.insert(id); } }
                self.last_selected_node = Some(id);
            }
            if let Some(id) = node_to_drag { self.dragged_node = Some(id); }
            if let Some(drag_id) = self.dragged_node {
                let delta = drag_delta / self.zoom;
                if self.selected_nodes.contains(&drag_id) { for &sid in &self.selected_nodes { if let Some(sn) = self.nodes.get_mut(&sid) { sn.position += delta; } } } 
                else { if let Some(n) = self.nodes.get_mut(&drag_id) { n.position += delta; } }
            }

            if let Some(start) = self.selection_start {
                let r = Rect::from_two_pos(self.world_to_screen(start, canvas_rect), m_pos);
                painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(100, 150, 255, 30));
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_rgb(100, 150, 255)));
                if ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary)) {
                    let wr = Rect::from_two_pos(start, world_m_pos);
                    if !ui.input(|i| i.modifiers.ctrl) { self.selected_nodes.clear(); }
                    for (id, node) in &self.nodes { if wr.intersects(Rect::from_min_size(node.position, node.size)) { self.selected_nodes.insert(*id); } }
                    self.selection_start = None;
                }
            }

            if let Some(last) = self.swipe_last_pos { painter.line_segment([self.world_to_screen(last, canvas_rect), m_pos], Stroke::new(2.0, Color32::RED)); }
            if let Some((sn, sp_idx)) = self.active_drag_source {
                let p1 = self.world_to_screen(self.get_port_world_pos(sn, sp_idx, true), canvas_rect);
                let mut color = Color32::YELLOW;
                if let Some(node) = self.nodes.get(&sn) { if let Some(tmpl) = self.templates.iter().find(|t| t.name == node.template_name) { if let Some(port) = tmpl.outputs.get(sp_idx) { color = port.data_type.color(); } } }
                self.draw_connection(&painter, p1, m_pos, color);
            }

            if ui.input(|i| i.pointer.any_released()) {
                if let Some((sn, sp_idx)) = self.active_drag_source {
                    if let Some((dn, dp_idx, is_output)) = hover_port {
                        if !is_output && sn != dn { 
                            self.push_undo();
                            self.connections.push(Connection { from_node: sn, from_port: sp_idx, to_node: dn, to_port: dp_idx }); 
                        }
                    }
                }
                if let Some(t) = self.dragged_template.take() { if canvas_rect.contains(m_pos) { self.add_node(&t.name, world_m_pos - Vec2::new(75.0, 10.0)); } }
                self.dragged_node = None; self.active_drag_source = None; self.selection_start = None;
            }

            if let Some(t) = &self.dragged_template {
                let pr = Rect::from_center_size(m_pos, Vec2::new(150.0, 40.0) * self.zoom);
                painter.rect_filled(pr, 5.0 * self.zoom, self.get_category_color(&t.category).gamma_multiply(0.5));
                painter.text(m_pos, Align2::CENTER_CENTER, &t.name, FontId::proportional(13.0 * self.zoom), Color32::WHITE);
            }
            if canvas_resp.clicked() && node_to_select.is_none() && !ui.input(|i| i.modifiers.shift) { self.selected_nodes.clear(); }

            if let Some(pos) = self.context_menu_pos {
                let mut close_menu = false;
                egui::Area::new(ui.id().with("context_menu")).fixed_pos(pos).order(egui::Order::Foreground).show(ui.ctx(), |ui| {
                    egui::Frame::menu(ui.style()).show(ui, |ui| {
                        ui.set_width(120.0);
                        if ui.add_enabled(!self.selected_nodes.is_empty(), egui::Button::new("Delete")).clicked() {
                            self.push_undo();
                            for id in self.selected_nodes.drain() { self.nodes.remove(&id); self.connections.retain(|c| c.from_node != id && c.to_node != id); }
                            close_menu = true;
                        }
                        if ui.button("Reset View").clicked() { self.pan_offset = Vec2::ZERO; self.zoom = 1.0; close_menu = true; }
                    });
                });
                if close_menu || (ui.input(|i| i.pointer.any_pressed()) && !Rect::from_min_size(pos, Vec2::new(120.0, 50.0)).contains(m_pos)) { self.context_menu_pos = None; }
            }
        });
    }
    fn box_clone(&self) -> Box<dyn TabInstance> { Box::new(self.clone()) }
}

// ----------------------------------------------------------------------------
// Loader / Utilities
// ----------------------------------------------------------------------------

#[derive(Deserialize)]
struct NhdMetadata { namespace: String, #[serde(default)] definitions: Vec<NhdDefinition> }
#[derive(Deserialize)]
struct NhdDefinition { name: String, function_name: String, category: String, #[serde(default)] inputs: Vec<NhdPort>, #[serde(default)] outputs: Vec<NhdPort>, widget: Option<String> }
#[derive(Deserialize)]
struct NhdPort { name: String, #[serde(rename = "type")] port_type: String }

struct NodeLoader;
impl NodeLoader {
    fn load_from_paths(paths: &[PathBuf]) -> Vec<NodeTemplate> {
        let mut t = Vec::new();
        for p in paths {
            if p.is_file() {
                if let Some(list) = Self::load_file(p) { t.extend(list); }
            } else if p.is_dir() {
                if let Ok(es) = std::fs::read_dir(p) {
                    for e in es.flatten() {
                        if let Some(list) = Self::load_file(&e.path()) { t.extend(list); }
                    }
                }
            }
        }
        t
    }
    fn load_file(path: &Path) -> Option<Vec<NodeTemplate>> {
        let ext = path.extension()?.to_str()?;
        let content = std::fs::read_to_string(path).ok()?;
        match ext {
            "nhd" => Some(Self::parse_nhd(&content)),
            _ => None
        }
    }
    fn parse_nhd(c: &str) -> Vec<NodeTemplate> {
        let s = "/*NHD_METADATA"; let e = "NHD_METADATA*/";
        let start = match c.find(s) { Some(i) => i + s.len(), None => return vec![] };
        let end = match c.find(e) { Some(i) => i, None => return vec![] };
        let m: NhdMetadata = match toml::from_str(&c[start..end]) {
            Ok(m) => m,
            Err(_) => return vec![],
        };
        m.definitions.into_iter().map(|d| NodeTemplate {
            name: d.name,
            category: format!("{}/{}", m.namespace, d.category),
            widget_type: d.widget,
            inputs: d.inputs.into_iter().map(|p| Port { name: p.name, data_type: Self::map_t(&p.port_type) }).collect(),
            outputs: d.outputs.into_iter().map(|p| Port { name: p.name, data_type: Self::map_t(&p.port_type) }).collect(),
            rhai_fn_name: d.function_name,
            rhai_body: c.to_string() // 保存完整内容以供后续可能的动态调用
        }).collect()
    }
    fn map_t(t: &str) -> DataType { match t.to_lowercase().as_str() { "flow" => DataType::Flow, "string" => DataType::String, "number" | "int" => DataType::Number, "bool" => DataType::Bool, _ => DataType::Any } }
}

// ----------------------------------------------------------------------------
// Plugin Implementation
// ----------------------------------------------------------------------------

pub struct VisualFlowCanvasPlugin {
    package_paths: Vec<PathBuf>,
    template_cache: Vec<NodeTemplate>,
}

impl Plugin for VisualFlowCanvasPlugin {
    fn name(&self) -> &str { crate::plugins::PLUGIN_NAME_VISUAL_FLOW_CANVAS }

    fn try_open_file(&mut self, path: &Path) -> Option<Box<dyn TabInstance>> {
        let ext = path.extension()?.to_str()?;
        if ext == "vfrh" {
            let content = std::fs::read_to_string(path).ok()?;
            if let Ok((nodes, connections)) = compiler::FlowLoader::parse(&content, &self.template_cache) {
                let mut tab = VisualFlowCanvasTab::new(self.template_cache.clone());
                tab.nodes = nodes;
                tab.connections = connections;
                tab.next_node_id = tab.nodes.keys().map(|id| id.0).max().unwrap_or(0) + 1;
                return Some(Box::new(tab));
            }
        }
        None
    }

    fn on_tab_menu(&mut self, ui: &mut Ui, control: &mut Vec<AppCommand>) {
        if ui.button("New VisualFlow Canvas").clicked() {
            control.push(AppCommand::OpenTab(Tab::new(Box::new(VisualFlowCanvasTab::new(self.template_cache.clone())))));
            ui.close_menu();
        }
    }

    fn on_settings_ui(&mut self, ui: &mut Ui) {
        ui.heading("Node Packages");
        ui.label(format!("Loaded {} templates", self.template_cache.len()));
        if ui.button("Reload All").clicked() { self.reload(); }
    }
}

impl VisualFlowCanvasPlugin {
    fn reload(&mut self) {
        self.template_cache = NodeLoader::load_from_paths(&self.package_paths);
        if self.template_cache.is_empty() {
            let core = "/*NHD_METADATA\nnamespace = \"Flow\"\n[[definitions]]\nname = \"Start\"\nfunction_name = \"start\"\ncategory = \"Core\"\noutputs = [{ name = \"next\", type = \"Flow\" }]\n[[definitions]]\nname = \"If\"\nfunction_name = \"branch\"\ncategory = \"Control\"\ninputs = [{ name = \"in\", type = \"Flow\" }, { name = \"condition\", type = \"Bool\" }]\noutputs = [{ name = \"true\", type = \"Flow\" }, { name = \"false\", type = \"Flow\" }]\nNHD_METADATA*/";
            self.template_cache.extend(NodeLoader::parse_nhd(core));
        }
    }
}

pub fn create() -> VisualFlowCanvasPlugin {
    let mut p = VisualFlowCanvasPlugin { package_paths: vec![PathBuf::from("src/plugins/visual_flow_canvas/nodes")], template_cache: Vec::new() };
    p.reload();
    p
}