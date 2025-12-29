use super::{NodeId, NodeInstance, Connection, NodeTemplate, DataType};
use std::collections::{HashMap, HashSet};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
struct BpMetadata {
    version: String,
    dependencies: Vec<String>,
    nodes: Vec<NodeMetadata>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
enum InputValue {
    Connection(NodeId),      // 引用其他节点的 ID
    Constant(String),       // 常量，例如 "constant:Hello"
}

#[derive(Serialize, Deserialize)]
struct NodeMetadata {
    id: usize,
    template: String,
    x: f32,
    y: f32,
    #[serde(default)]
    inputs: Vec<InputValue>, 
    #[serde(default)]
    flows: Vec<isize>, 
}

pub struct FlowCompiler<'a> {
    nodes: &'a HashMap<NodeId, NodeInstance>,
    connections: &'a Vec<Connection>,
    templates: &'a [NodeTemplate],
    generated_outputs: HashMap<(NodeId, usize), String>,
    visited_data_nodes: HashSet<NodeId>,
    lines: Vec<String>,
}

impl<'a> FlowCompiler<'a> {
    pub fn new(nodes: &'a HashMap<NodeId, NodeInstance>, connections: &'a Vec<Connection>, templates: &'a [NodeTemplate]) -> Self {
        Self { 
            nodes, 
            connections, 
            templates, 
            generated_outputs: HashMap::new(),
            visited_data_nodes: HashSet::new(),
            lines: Vec::new(),
        }
    }

    pub fn compile(&mut self) -> Result<String, String> {
        let start_node = self.nodes.values() 
            .find(|n| n.template_name == "Start")
            .ok_or("No 'Start' node found.")?;

        self.lines.push("// --- 自动生成的逻辑流 ---".to_string());
        self.lines.push("import \"std\" as std;".to_string());
        self.lines.push("".to_string());

        self.lines.push("fn main_logic(context) {".to_string());
        self.compile_flow_chain(start_node.id, 1)?;
        self.lines.push("}".to_string());
        self.lines.push("".to_string());

        let mut metadata = BpMetadata {
            version: "1.0".to_string(),
            dependencies: vec!["std".to_string()],
            nodes: Vec::new(),
        };

        for node in self.nodes.values() {
            let template = self.templates.iter().find(|t| t.name == node.template_name).unwrap();
            let mut inputs = Vec::new();

            // 1. 记录数据输入
            for (i, port) in template.inputs.iter().enumerate() {
                if port.data_type == DataType::Flow { continue; }
                if let Some(conn) = self.connections.iter().find(|c| c.to_node == node.id && c.to_port == i) {
                    inputs.push(InputValue::Connection(conn.from_node));
                } else {
                    // 如果没有连接，对于普通的带有输入端口的节点，记录其关联的 data (虽然通常这种节点不会直接用 data)
                    inputs.push(InputValue::Constant(format!("constant:{}", node.data)));
                }
            }
            
            // 2. 特殊处理：如果没有普通输入端口但是有 widget (如 String Literal 节点)，
            // 我们必须确保其 node.data 被记录到 inputs 元数据中，否则加载时会丢失内容。
            let has_data_input = template.inputs.iter().any(|p| p.data_type != DataType::Flow);
            if !has_data_input && template.widget_type.is_some() {
                // 此时 inputs 列表应该是空的，我们加入唯一的常量数据
                inputs.push(InputValue::Constant(format!("constant:{}", node.data)));
            }

            let mut flows = Vec::new();
            for (i, port) in template.outputs.iter().enumerate() {
                if port.data_type == DataType::Flow {
                    let target = self.connections.iter()
                        .find(|c| c.from_node == node.id && c.from_port == i)
                        .map(|c| c.to_node.0 as isize)
                        .unwrap_or(-1);
                    flows.push(target);
                }
            }

            metadata.nodes.push(NodeMetadata {
                id: node.id.0,
                template: node.template_name.clone(),
                x: node.position.x,
                y: node.position.y,
                inputs,
                flows,
            });
        }

        let toml_str = toml::to_string_pretty(&metadata).map_err(|e| e.to_string())?;
        self.lines.push("/*BP_METADATA".to_string());
        self.lines.push(toml_str);
        self.lines.push("BP_METADATA*/".to_string());

        Ok(self.lines.join("\n"))
    }

    fn compile_flow_chain(&mut self, start_node_id: NodeId, indent: usize) -> Result<(), String> {
        let mut current_node_id = Some(start_node_id);
        let indent_str = "    ".repeat(indent);
        let mut steps = 0;

        while let Some(node_id) = current_node_id {
            if steps > 2000 { return Err("Flow loop detected".into()); }
            steps += 1;

            let node = self.nodes.get(&node_id).ok_or("Node not found")?;
            let template = self.templates.iter().find(|t| t.name == node.template_name).ok_or("Template missing")?;

            if template.name == "Start" {
                if let Some(idx) = template.outputs.iter().position(|p| p.data_type == DataType::Flow) {
                    current_node_id = self.get_connected_target(node_id, idx, true);
                    continue;
                }
                break;
            }

            self.lines.push(format!("{}// 节点 ID:{} - {}", indent_str, node.id.0, node.template_name));
            
            if template.name == "If" {
                let args = self.resolve_inputs(node_id)?;
                let cond = args.get(0).cloned().unwrap_or("false".to_string());
                self.lines.push(format!("{}if {} {{", indent_str, cond));
                if let Some(t) = self.get_connected_target(node_id, 0, true) { self.compile_flow_chain(t, indent + 1)?; }
                self.lines.push(format!("{}}} else {{", indent_str));
                if let Some(f) = self.get_connected_target(node_id, 1, true) { self.compile_flow_chain(f, indent + 1)?; }
                self.lines.push(format!("{}}}", indent_str));
                break; 
            } else {
                let code = self.generate_execution_code(node_id)?;
                for line in code { self.lines.push(format!("{}{}", indent_str, line)); }
                if let Some(idx) = template.outputs.iter().position(|p| p.data_type == DataType::Flow) {
                    current_node_id = self.get_connected_target(node_id, idx, true);
                } else {
                    current_node_id = None;
                }
            }
        }
        Ok(())
    }

    fn get_connected_target(&self, node_id: NodeId, port_index: usize, is_out: bool) -> Option<NodeId> {
        if is_out {
            self.connections.iter().find(|c| c.from_node == node_id && c.from_port == port_index).map(|c| c.to_node)
        } else {
            self.connections.iter().find(|c| c.to_node == node_id && c.to_port == port_index).map(|c| c.from_node)
        }
    }

    fn resolve_inputs(&mut self, node_id: NodeId) -> Result<Vec<String>, String> {
        let node = self.nodes.get(&node_id).unwrap();
        let template = self.templates.iter().find(|t| t.name == node.template_name).unwrap();
        let mut args = Vec::new();

        for (i, port) in template.inputs.iter().enumerate() {
            if port.data_type == DataType::Flow { continue; }
            if let Some(conn) = self.connections.iter().find(|c| c.to_node == node_id && c.to_port == i) {
                self.ensure_data_ready(conn.from_node)?;
                if let Some(var) = self.generated_outputs.get(&(conn.from_node, conn.from_port)) {
                    args.push(var.clone());
                }
            } else {
                args.push(match port.data_type {
                    DataType::String => format!("{:?}", node.data),
                    DataType::Number => if node.data.is_empty() { "0".to_string() } else { node.data.clone() },
                    DataType::Bool => if node.data == "true" { "true".to_string() } else { "false".to_string() },
                    _ => "\"\"".to_string()
                });
            }
        }
        Ok(args)
    }

    fn ensure_data_ready(&mut self, node_id: NodeId) -> Result<(), String> {
        if self.visited_data_nodes.contains(&node_id) { return Ok(()); }
        let node = self.nodes.get(&node_id).unwrap();
        let template = self.templates.iter().find(|t| t.name == node.template_name).unwrap();
        if template.inputs.iter().all(|p| p.data_type != DataType::Flow) {
            let _ = self.resolve_inputs(node_id)?;
            let code = self.generate_execution_code(node_id)?;
            let indent = "    ";
            for line in code { self.lines.push(format!("{}{}", indent, line)); }
            self.visited_data_nodes.insert(node_id);
        }
        Ok(())
    }

    fn generate_execution_code(&mut self, node_id: NodeId) -> Result<Vec<String>, String> {
        let node = self.nodes.get(&node_id).unwrap();
        let template = self.templates.iter().find(|t| t.name == node.template_name).unwrap();
        let args = self.resolve_inputs(node_id)?;
        let mut lines = Vec::new();

        let var_name = format!("_t{}", node_id.0);
        
        // 识别是否为“字面量/常量”节点：没有数据输入端口，但有 Widget 且有数据。
        let is_literal = template.inputs.iter().all(|p| p.data_type == DataType::Flow) && template.widget_type.is_some();

        let fn_call = if is_literal {
            // 直接根据输出类型格式化常量，{:?} 会处理字符串转义（如路径中的 \）
            match template.outputs.first().map(|p| &p.data_type) {
                Some(DataType::String) => format!("{:?}", node.data),
                Some(DataType::Number) => if node.data.is_empty() { "0".to_string() } else { node.data.clone() },
                Some(DataType::Bool) => if node.data == "true" { "true".to_string() } else { "false".to_string() },
                _ => format!("{:?}", node.data)
            }
        } else if template.rhai_fn_name.is_empty() {
            args.get(0).cloned().unwrap_or("\"\"".to_string())
        } else {
            format!("{}({})", template.rhai_fn_name, args.join(", "))
        };

        if template.outputs.iter().any(|p| p.data_type != DataType::Flow) {
            lines.push(format!("let {} = {};", var_name, fn_call));
            if let Some(idx) = template.outputs.iter().position(|p| p.data_type != DataType::Flow) {
                self.generated_outputs.insert((node_id, idx), var_name);
            }
        } else {
            lines.push(format!("{};", fn_call));
        }
        Ok(lines)
    }
}

pub struct FlowLoader;

impl FlowLoader {
    pub fn parse(content: &str, templates: &[NodeTemplate]) -> Result<(HashMap<NodeId, NodeInstance>, Vec<Connection>), String> {
        let start_tag = "/*BP_METADATA";
        let end_tag = "BP_METADATA*/";

        let start_idx = content.find(start_tag).ok_or("No BP_METADATA found")? + start_tag.len();
        let end_idx = content.find(end_tag).ok_or("End of BP_METADATA missing")?;
        let toml_part = &content[start_idx..end_idx].trim();

        let metadata: BpMetadata = toml::from_str(toml_part).map_err(|e| e.to_string())?;

        let mut nodes = HashMap::new();
        let mut connections = Vec::new();

        for n in &metadata.nodes {
            let template = templates.iter().find(|t| t.name == n.template).ok_or(format!("Template {} not found", n.template))?;
            let max_ports = template.inputs.len().max(template.outputs.len());
            let widget_h = if template.widget_type.is_some() { 25.0 } else { 0.0 };
            let height = 35.0 + widget_h + (max_ports as f32 * 22.0).max(22.0);

            let data = n.inputs.iter().filter_map(|iv| {
                if let InputValue::Constant(c) = iv {
                    Some(c.strip_prefix("constant:").unwrap_or(c.as_str()).to_string())
                } else {
                    None
                }
            }).collect::<Vec<_>>().join(";");

            nodes.insert(NodeId(n.id), NodeInstance {
                id: NodeId(n.id),
                template_name: n.template.clone(),
                position: super::Pos2::new(n.x, n.y),
                size: super::Vec2::new(150.0, height),
                data,
            });
        }

        for n in &metadata.nodes {
            let template_opt = templates.iter().find(|t| t.name == n.template);
            if template_opt.is_none() { continue; }
            let template = template_opt.unwrap();
            
            let data_inputs: Vec<usize> = template.inputs.iter().enumerate()
                .filter(|(_, p)| p.data_type != DataType::Flow)
                .map(|(i, _)| i)
                .collect();

            for (i, iv) in n.inputs.iter().enumerate() {
                if let &InputValue::Connection(from_node_id) = iv {
                    if let Some(&to_port) = data_inputs.get(i) {
                        let from_node_instance = nodes.get(&from_node_id).ok_or(format!("Source node {:?} not found", from_node_id))?;
                        let from_template = templates.iter().find(|t| t.name == from_node_instance.template_name)
                            .ok_or(format!("Template {} not found", from_node_instance.template_name))?;
                        let from_port = from_template.outputs.iter().position(|p| p.data_type != DataType::Flow).unwrap_or(0);

                        connections.push(Connection {
                            from_node: from_node_id,
                            from_port,
                            to_node: NodeId(n.id),
                            to_port,
                        });
                    }
                }
            }

            let flow_outputs: Vec<usize> = template.outputs.iter().enumerate()
                .filter(|(_, p)| p.data_type == DataType::Flow)
                .map(|(i, _)| i)
                .collect();

            for (i, &next_id) in n.flows.iter().enumerate() {
                if next_id != -1 {
                    let next_node_id = NodeId(next_id as usize);
                    if let Some(&from_port) = flow_outputs.get(i) {
                        if let Some(next_node) = nodes.get(&next_node_id) {
                            let next_template = templates.iter().find(|t| t.name == next_node.template_name).unwrap();
                            let to_port = next_template.inputs.iter().position(|p| p.data_type == DataType::Flow).unwrap_or(0);

                            connections.push(Connection {
                                from_node: NodeId(n.id),
                                from_port,
                                to_node: next_node_id,
                                to_port,
                            });
                        }
                    }
                }
            }
        }

        Ok((nodes, connections))
    }
}
