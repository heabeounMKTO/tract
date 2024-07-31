use tract_onnx::prelude::*;
use tract_ndarray::{Array,s, Array3};
use clap::Parser;
use anyhow::{Error, Result};
use image::DynamicImage;
use std::cmp::Ordering;
use std::cmp::{PartialOrd};




#[derive(Parser)]
struct CliArgs {
    #[arg(long)]
    input_image: String,
    
    #[arg(long)]
    weights: String,
}

fn scale_wh(w0: f32, h0: f32, w1: f32, h1: f32) -> (f32, f32, f32) {
    let r = (w1 / w0).min(h1 / h0);
    (r, (w0 * r).round(), (h0 * r).round())
}

#[derive(Debug, Clone)]
pub struct Bbox {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub confidence: f32,
    pub class_index: i32
}

impl Bbox {
    pub fn new(x1: f32, y1: f32, x2: f32, y2: f32, confidence: f32, class_index: i32) -> Bbox {
        Bbox {
            x1,
            y1,
            x2,
            y2,
            confidence,
            class_index
        }
    }
    pub fn apply_image_scale(
        &mut self,
        original_image: &DynamicImage,
        x_scale: f32,
        y_scale: f32,
    ) -> Bbox {
        let normalized_x1 = self.x1 / x_scale;
        let normalized_x2 = self.x2 / x_scale;
        let normalized_y1 = self.y1 / y_scale;
        let normalized_y2 = self.y2 / y_scale;

        let cart_x1 = original_image.width() as f32 * normalized_x1;
        let cart_x2 = original_image.width() as f32 * normalized_x2;
        let cart_y1 = original_image.height() as f32 * normalized_y1;
        let cart_y2 = original_image.height() as f32 * normalized_y2;

        Bbox {
            x1: cart_x1,
            y1: cart_y1,
            x2: cart_x2,
            y2: cart_y2,
            confidence: self.confidence,
            class_index: self.class_index
        }
    }
}


pub fn non_maximum_suppression(mut boxes: Vec<Bbox>, iou_threshold: f32) -> Vec<Bbox> {
    boxes.sort_by(|a, b| {
        a.confidence
            .partial_cmp(&b.confidence)
            .unwrap_or(Ordering::Equal)
    });
    let mut keep = Vec::new();
    while !boxes.is_empty() {
        let current = boxes.remove(0);
        keep.push(current.clone());
        boxes.retain(|box_| calculate_iou(&current, box_) <= iou_threshold);
    }

    keep
}

fn calculate_iou(box1: &Bbox, box2: &Bbox) -> f32 {
    let x1 = box1.x1.max(box2.x1);
    let y1 = box1.y1.max(box2.y1);
    let x2 = box1.x2.min(box2.x2);
    let y2 = box1.y2.min(box2.y2);

    let intersection = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
    let area1 = (box1.x2 - box1.x1) * (box1.y2 - box1.y1);
    let area2 = (box2.x2 - box2.x1) * (box2.y2 - box2.y1);
    let union = area1 + area2 - intersection;
    intersection / union
}



fn main() -> Result<(), Error> {
    let args = CliArgs::parse();
    let model = tract_onnx::onnx()
        .model_for_path(args.weights)?
        .with_input_fact(0, f32::fact([1,3,640,640]).into())?
        .into_optimized()?
        .into_runnable()?;
    let raw_image = image::open(args.input_image)?;

    let (_, w_new, h_new) = scale_wh(raw_image.width() as f32,
                                      raw_image.height() as f32, 
                                    640.0,640.0);
    let resized = image::imageops::resize(&raw_image.to_rgb8(), w_new as u32, h_new as u32 , image::imageops::FilterType::Triangle);
    let image: Tensor = tract_ndarray::Array4::from_shape_fn((1, 3, 640, 640), |(_, c, y, x)| {
        resized[(x as _, y as _)][c] as f32 / 255.0
    })
    .into();    
    let forward = model.run(tvec![image.to_owned().into()])?;
    let results = forward[0].to_array_view::<f32>()?.view().t().into_owned();
    let mut bbox_vec: Vec<Bbox> = vec![];
    for i in 0..results.len_of(tract_ndarray::Axis(0)) {
        let row = results.slice(s![i, .., ..]);
        let confidence = row[[4, 0]];

        let mut max_prob = 0.0;
        let mut class_index = 0;
        for j in 0..80 {  
            // iterate over the total yolo classes, 
            // in our case, 80, TODO: make it dynamic?!
            let prob = row[[j + 5, 0]];
            if prob > max_prob {
                max_prob = prob;
                class_index = j;
            }
        }

        if &confidence >= &0.5 {
            let x = row[[0, 0]];
            let y = row[[1, 0]];
            let w = row[[2, 0]];
            let h = row[[3, 0]];
            let x1 = x - w / 2.0;
            let y1 = y - h / 2.0;
            let x2 = x + w / 2.0;
            let y2 = y + h / 2.0;
            let bbox = Bbox::new(x1, y1, x2, y2, confidence, class_index as i32).apply_image_scale(
                &raw_image,
                w_new as f32,
                h_new as f32,
            );
            bbox_vec.push(bbox);
        
        }
    }    
    println!("bboxes: {:?}", bbox_vec);
    Ok(())
}

