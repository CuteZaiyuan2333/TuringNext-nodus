use super::logic::{GateType, LogicState};
use egui::Color32;
use egui_node_graph2::*;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

// ----------------------------------------------------------------------------
// Types and Data
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MyDataType {
    Logic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MyValueType {
    Logic(LogicState),
}

impl Default for MyValueType {
    fn default() -> Self {
        Self::Logic(LogicState::Low)
    }
}

impl WidgetValueTrait for MyValueType {
    type Response = MyResponse;
    type UserState = MyGraphState;
    type NodeData = MyNodeData;
    fn value_widget(
        &mut self,
        _param_name: &str,
        _node_id: NodeId,
        ui: &mut egui::Ui,
        _user_state: &mut MyGraphState,
        _node_data: &MyNodeData,
    ) -> Vec<MyResponse> {
        match self {
            MyValueType::Logic(s) => {
                if ui.button(format!("{:?}", s)).clicked() {
                    *s = s.inverted();
                }
            }
        }
        vec![]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MyNodeData {
    pub gate_type: GateType,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MyGraphState {
    pub active_simulation: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MyResponse {
    SetActive(bool),
}

impl UserResponseTrait for MyResponse {}

// ----------------------------------------------------------------------------
// Traits Implementation
// ----------------------------------------------------------------------------

impl DataTypeTrait<MyGraphState> for MyDataType {
    fn data_type_color(&self, _user_state: &mut MyGraphState) -> Color32 {
        Color32::from_rgb(100, 200, 100)
    }

    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed("Logic")
    }
}

impl NodeDataTrait for MyNodeData {
    type Response = MyResponse;
    type UserState = MyGraphState;
    type DataType = MyDataType;
    type ValueType = MyValueType;

    fn can_delete(
        &self,
        _node_id: NodeId,
        _graph: &Graph<Self, Self::DataType, Self::ValueType>,
        _user_state: &mut Self::UserState,
    ) -> bool {
        true
    }

    fn bottom_ui(
        &self,
        _ui: &mut egui::Ui,
        _node_id: NodeId,
        _graph: &Graph<Self, Self::DataType, Self::ValueType>,
        _user_state: &mut Self::UserState,
    ) -> Vec<NodeResponse<Self::Response, Self>> {
        vec![]
    }
}

// ----------------------------------------------------------------------------
// Templates
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MyNodeTemplate {
    And,
    Or,
    Not,
    Xor,
    Input,
    Output,
}

impl NodeTemplateIter for MyNodeTemplate {
    type Item = MyNodeTemplate;

    fn all_kinds(&self) -> Vec<Self::Item> {
        vec![
            MyNodeTemplate::And,
            MyNodeTemplate::Or,
            MyNodeTemplate::Not,
            MyNodeTemplate::Xor,
            MyNodeTemplate::Input,
            MyNodeTemplate::Output,
        ]
    }
}

impl NodeTemplateTrait for MyNodeTemplate {
    type NodeData = MyNodeData;
    type DataType = MyDataType;
    type ValueType = MyValueType;
    type UserState = MyGraphState;
    type CategoryType = &'static str;

    fn node_finder_label(&self, _user_state: &mut Self::UserState) -> Cow<'_, str> {
        Cow::Owned(format!("{:?}", self))
    }

    fn node_graph_label(&self, _user_state: &mut Self::UserState) -> String {
        format!("{:?}", self)
    }

    fn user_data(&self, _user_state: &mut Self::UserState) -> Self::NodeData {
        MyNodeData {
            gate_type: match self {
                MyNodeTemplate::And => GateType::And,
                MyNodeTemplate::Or => GateType::Or,
                MyNodeTemplate::Not => GateType::Not,
                MyNodeTemplate::Xor => GateType::Xor,
                MyNodeTemplate::Input => GateType::Input,
                MyNodeTemplate::Output => GateType::Output,
            },
        }
    }

    fn build_node(
        &self,
        graph: &mut Graph<Self::NodeData, Self::DataType, Self::ValueType>,
        _user_state: &mut Self::UserState,
        node_id: NodeId,
    ) {
        match self {
            MyNodeTemplate::And | MyNodeTemplate::Or | MyNodeTemplate::Xor => {
                graph.add_input_param(node_id, "In A".into(), MyDataType::Logic, MyValueType::Logic(LogicState::Low), InputParamKind::ConnectionOrConstant, true);
                graph.add_input_param(node_id, "In B".into(), MyDataType::Logic, MyValueType::Logic(LogicState::Low), InputParamKind::ConnectionOrConstant, true);
                graph.add_output_param(node_id, "Out".into(), MyDataType::Logic);
            }
            MyNodeTemplate::Not => {
                graph.add_input_param(node_id, "In".into(), MyDataType::Logic, MyValueType::Logic(LogicState::Low), InputParamKind::ConnectionOrConstant, true);
                graph.add_output_param(node_id, "Out".into(), MyDataType::Logic);
            }
            MyNodeTemplate::Input => {
                graph.add_output_param(node_id, "Value".into(), MyDataType::Logic);
            }
            MyNodeTemplate::Output => {
                graph.add_input_param(node_id, "Value".into(), MyDataType::Logic, MyValueType::Logic(LogicState::Low), InputParamKind::ConnectionOrConstant, true);
            }
        }
    }
}

pub type MyGraph = Graph<MyNodeData, MyDataType, MyValueType>;
pub type MyEditorState = GraphEditorState<MyNodeData, MyDataType, MyValueType, MyNodeTemplate, MyGraphState>;
