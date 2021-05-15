//! Extract information from glsl source and transpiles it to valid glsl source code.

use core::panic;
use std::borrow::Borrow;
use std::ops::Deref;

use anyhow::{anyhow, Result};
use glsl_lang::ast::{FunIdentifier, PreprocessorDefine, TypeSpecifier, TypeSpecifierNonArray};
use glsl_lang::{
    ast::{
        Block, Expr, Identifier, IdentifierData, LayoutQualifier, LayoutQualifierSpec, SmolStr,
        StructFieldSpecifier, TranslationUnit, TypeQualifierSpec,
    },
    parse::{Parsable, ParseOptions},
    transpiler::glsl::FormattingState,
    visitor::{HostMut, Visit, VisitorMut},
};
use log::{debug, error};
use mint::Vector3;

use crate::shader::{ShaderMetadata, Slider};

impl VisitorMut for ShaderMetadata {
    fn visit_block(&mut self, block: &mut Block) -> Visit {
        if let Some(TypeQualifierSpec::Layout(layout)) = block.qualifier.qualifiers.first() {
            if let Some(LayoutQualifierSpec::Identifier(id, _)) = layout.ids.first() {
                if id.content.0 == "params" {
                    // We got the block we searched for
                    for field in block.fields.iter_mut() {
                        if let Ok(slider) = create_slider_from_field(field) {
                            self.sliders.push(slider);
                            convert_field(field);
                        } else {
                            panic!("Invalid field");
                        }
                    }
                    convert_block(block);
                }
            }
        }
        Visit::Parent
    }

    fn visit_preprocessor_define(&mut self, define: &mut PreprocessorDefine) -> Visit {
        if let PreprocessorDefine::ObjectLike { ident, .. } = define {
            if ident.content.0.as_str() == "NUANCE_STILL_IMAGE" {
                self.still_image = true;
            }
        }
        Visit::Parent
    }

    fn visit_expr(&mut self, expr: &mut Expr) -> Visit {
        if let Expr::Dot(expr2, ident1) = expr {
            if let Expr::Variable(ident0) = expr2.as_ref() {
                let slider_name = ident0.content.0.as_str();
                for slider in self.sliders.iter() {
                    match slider {
                        Slider::Float { name, min, max, .. } => {
                            if name == slider_name {
                                *expr = Expr::FloatConst(match ident1.content.0.as_str() {
                                    "max" => *max,
                                    "min" => *min,
                                    _ => panic!("No such field exist !"),
                                });
                                return Visit::Parent;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        Visit::Children
    }
}

pub fn create_slider_from_field(field: &StructFieldSpecifier) -> Result<Slider> {
    let name = field
        .identifiers
        .first()
        .unwrap()
        .ident
        .content
        .0
        .to_string();

    //debug!("{:#?}", field);

    match field.ty.ty {
        // To Slider::Float
        TypeSpecifierNonArray::Float => {
            let mut min = 0.0;
            let mut max = 1.0;
            let mut init = 0.0;
            if let TypeQualifierSpec::Layout(LayoutQualifier { ids }) = field
                .qualifier
                .as_ref()
                .unwrap()
                .qualifiers
                .first()
                .unwrap()
            {
                for qualifier in ids.iter() {
                    if let LayoutQualifierSpec::Identifier(id, param) = qualifier {
                        match id.content.0.as_str() {
                            "min" => {
                                min = param.as_ref().unwrap().coerce_const();
                            }
                            "max" => {
                                max = param.as_ref().unwrap().coerce_const();
                            }
                            "init" => {
                                init = param.as_ref().unwrap().coerce_const();
                            }
                            other => {
                                error!("Wrong slider setting : {}", other)
                            }
                        }
                    }
                }
            }
            return Ok(Slider::Float {
                name,
                min,
                max,
                value: init,
            });
        }
        // To Slider::Color if color layout qualifier is set
        TypeSpecifierNonArray::Vec3 => {
            if let TypeQualifierSpec::Layout(LayoutQualifier { ids }) = field
                .qualifier
                .as_ref()
                .unwrap()
                .qualifiers
                .first()
                .unwrap()
            {
                let mut value: Vector3<f32> = Vector3::from([0.0, 0.0, 0.0]);
                let mut color = false;

                for qualifier in ids.iter() {
                    if let LayoutQualifierSpec::Identifier(id, param) = qualifier {
                        match id.content.0.as_str() {
                            "color" => {
                                color = true;
                            }
                            "init" => {
                                if let Some(Expr::FunCall(
                                    FunIdentifier::TypeSpecifier(TypeSpecifier { ty, .. }),
                                    params,
                                )) = param.as_deref()
                                {
                                    if *ty == TypeSpecifierNonArray::Vec3 && params.len() == 3 {
                                        value = Vector3::from([
                                            params[0].coerce_const(),
                                            params[1].coerce_const(),
                                            params[2].coerce_const(),
                                        ]);
                                        continue;
                                    }
                                    error!("Invalid initializer !");
                                }
                            }
                            other => {
                                error!("Unsupported setting : {}", other)
                            }
                        }
                    } else {
                        error!("Invalid qualifier shared");
                    }
                }
                return Ok(if color {
                    Slider::Color { name, value }
                } else {
                    Slider::Vec3 { name, value }
                });
            }
        }
        _ => {}
    }
    Err(anyhow!("Invalid field in params block"))
}

/// Replace the layout(params) with a predefined layout(set=?, binding=?)
pub fn convert_block(block: &mut Block) {
    block.qualifier.qualifiers[0] = TypeQualifierSpec::Layout(LayoutQualifier {
        ids: vec![
            LayoutQualifierSpec::Identifier(
                Identifier {
                    content: IdentifierData(SmolStr::new("set")),
                    span: None,
                },
                Some(Box::new(Expr::IntConst(0))),
            ),
            LayoutQualifierSpec::Identifier(
                Identifier {
                    content: IdentifierData(SmolStr::new("binding")),
                    span: None,
                },
                Some(Box::new(Expr::IntConst(0))),
            ),
        ],
    });
}

/// Replace the layout(min=?, max=?) with nothing
pub fn convert_field(field: &mut StructFieldSpecifier) {
    field.qualifier = None;
}

pub fn extract(source: &str) -> Result<(ShaderMetadata, String)> {
    let mut metadata = ShaderMetadata::default();

    // The AST
    let (mut ast, _ctx) = TranslationUnit::parse_with_options(
        source,
        &ParseOptions {
            target_vulkan: true,
            source_id: 0,
            allow_rs_ident: false,
        }
        .build(),
    )?;

    // Extract some ast juice
    ast.visit_mut(&mut metadata);

    let mut transpiled = String::new();
    glsl_lang::transpiler::glsl::show_translation_unit(
        &mut transpiled,
        &ast,
        FormattingState::default(),
    )?;
    Ok((metadata, transpiled))
}

trait CoerceConst<T> {
    fn coerce_const(&self) -> T;
}

impl<T> CoerceConst<f32> for T
where
    T: Borrow<Expr>,
{
    fn coerce_const(&self) -> f32 {
        match self.borrow() {
            Expr::IntConst(value) => *value as f32,
            Expr::UIntConst(value) => *value as f32,
            Expr::FloatConst(value) => *value,
            Expr::DoubleConst(value) => *value as f32,
            _ => {
                panic!("Not a number constant")
            }
        }
    }
}

impl CoerceConst<f64> for Expr {
    fn coerce_const(&self) -> f64 {
        match self {
            Expr::IntConst(value) => *value as f64,
            Expr::UIntConst(value) => *value as f64,
            Expr::FloatConst(value) => *value as f64,
            Expr::DoubleConst(value) => *value,
            _ => {
                panic!("Not a number constant")
            }
        }
    }
}

impl CoerceConst<i32> for Expr {
    fn coerce_const(&self) -> i32 {
        match self {
            Expr::IntConst(value) => *value,
            Expr::UIntConst(value) => *value as i32,
            Expr::FloatConst(value) => *value as i32,
            Expr::DoubleConst(value) => *value as i32,
            _ => {
                panic!("Not a number constant")
            }
        }
    }
}

impl CoerceConst<u32> for Expr {
    fn coerce_const(&self) -> u32 {
        match self {
            Expr::IntConst(value) => *value as u32,
            Expr::UIntConst(value) => *value,
            Expr::FloatConst(value) => *value as u32,
            Expr::DoubleConst(value) => *value as u32,
            _ => {
                panic!("Not a number constant")
            }
        }
    }
}
