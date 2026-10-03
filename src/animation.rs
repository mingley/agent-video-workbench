//! Render the geometry channels the pinned compiler otherwise freezes.
use crate::{Error, Result};
use agentcut_core::{Interpolation, ItemPayload, KeyframeChannel, Project, RationalTime};
use agentcut_render::{compile::RenderPlan, ir::RenderIr};

fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}
fn expression(channel: &KeyframeChannel, axis: Option<&str>, time: &str) -> Result<String> {
    let value = |i: usize| -> Result<f64> {
        let v = &channel.points[i].value;
        axis.map_or(v.as_f64(), |a| v[a].as_f64())
            .filter(|v| v.is_finite())
            .ok_or_else(|| invalid("animation value is not finite numeric data"))
    };
    if channel.points.is_empty() {
        return Err(invalid("animation channel has no points"));
    }
    let mut result = value(channel.points.len() - 1)?.to_string();
    for i in (0..channel.points.len().saturating_sub(1)).rev() {
        let start = channel.points[i].time.as_seconds_f64();
        let end = channel.points[i + 1].time.as_seconds_f64();
        if end <= start {
            return Err(invalid("animation points require increasing times"));
        }
        let a = value(i)?;
        let b = value(i + 1)?;
        if channel.points[i].easing.is_some() {
            return Err(invalid(
                "geometry animation supports linear/step interpolation without easing",
            ));
        }
        let segment = match channel.points[i].interpolation {
            Interpolation::Step => a.to_string(),
            Interpolation::Linear => format!("{a}+({b}-{a})*({time}-{start})/({end}-{start})"),
            Interpolation::Bezier => {
                return Err(invalid(
                    "Bezier geometry animation is outside the supported render path",
                ));
            }
        };
        result = format!("if(lt({time},{end}),if(lt({time},{start}),{a},{segment}),{result})");
    }
    Ok(result)
}
pub fn validate(project: &Project, sequence: &str) -> Result<()> {
    let selected = project.require_sequence(sequence)?;
    if selected
        .tracks
        .iter()
        .any(|t| t.effects.iter().any(|e| e.enabled))
    {
        return Err(invalid(
            "track effects require explicit item effects or an audio bus; this compiler does not apply track effects",
        ));
    }
    if selected
        .tracks
        .iter()
        .flat_map(|t| &t.effects)
        .chain(selected.buses.iter().flat_map(|b| &b.effects))
        .any(|e| !e.keyframes.is_empty())
    {
        return Err(invalid(
            "track/bus effect parameter animation is outside the supported renderer",
        ));
    }
    for item in project
        .require_sequence(sequence)?
        .tracks
        .iter()
        .flat_map(|t| &t.items)
    {
        for channel in &item.keyframes {
            if !matches!(
                channel.property.as_str(),
                "opacity"
                    | "transform.position"
                    | "video.crop.left"
                    | "video.crop.right"
                    | "video.crop.top"
                    | "video.crop.bottom"
            ) {
                return Err(Error::Invalid(format!(
                    "item {}: keyframed {} has no qualified renderer",
                    item.id, channel.property
                )));
            }
        }
        if item
            .effects
            .iter()
            .any(|e| e.enabled && !e.keyframes.is_empty())
        {
            return Err(invalid(
                "effect parameter animation is outside the supported renderer",
            ));
        }
        for effect in item.effects.iter().filter(|e| e.enabled) {
            if effect.capability == "video.color.basic"
                && ["temperature", "tint", "highlights", "shadows"]
                    .iter()
                    .any(|p| {
                        effect
                            .parameters
                            .get(*p)
                            .and_then(serde_json::Value::as_f64)
                            .is_some_and(|v| v != 0.0)
                    })
            {
                return Err(invalid(
                    "basic grade supports brightness/exposure/contrast/saturation; use an explicit LUT for other color corrections",
                ));
            }
        }
    }
    Ok(())
}
pub fn adapt(plan: &mut RenderPlan, ir: &RenderIr, project: &Project) -> Result<()> {
    let position = plan
        .args
        .iter()
        .position(|a| a == "-filter_complex")
        .ok_or_else(|| invalid("animation graph missing"))?
        + 1;
    let graph = plan
        .args
        .get_mut(position)
        .ok_or_else(|| invalid("animation graph missing"))?;
    let mut elements: Vec<_> = ir.video_elements.iter().collect();
    elements.sort_by_key(|e| e.layer);
    for (ordinal, element) in elements.iter().enumerate() {
        let Some((_, _, item)) = project.find_item(&element.item_id) else {
            continue;
        };
        if item.keyframes.is_empty() {
            continue;
        }
        if element.blend_mode != agentcut_core::BlendMode::Normal || !ir.transitions.is_empty() {
            return Err(invalid(
                "animated framing requires normal blend and no simultaneous transition",
            ));
        }
        if let Some(channel) = item
            .keyframes
            .iter()
            .find(|c| c.property == "transform.position")
        {
            let anchor = match &item.payload {
                ItemPayload::Clip(c) => c.transform.position,
                ItemPayload::Text(t) => t.transform.position,
                _ => {
                    return Err(invalid(
                        "position animation currently supports clips and text",
                    ));
                }
            };
            let time = format!("(t-{})", item.start.as_seconds_f64());
            let x = expression(channel, Some("x"), &time)?;
            let y = expression(channel, Some("y"), &time)?;
            let needle = format!(
                "[v{ordinal}]overlay=x={}:y={}",
                element.placement.x.round() as i64,
                element.placement.y.round() as i64
            );
            let replacement = format!(
                "[v{ordinal}]overlay=x='({x})+{}-{}':y='({y})+{}-{}'",
                element.placement.x, anchor.x, element.placement.y, anchor.y
            );
            if !graph.contains(&needle) {
                return Err(invalid(
                    "animated overlay is incompatible with this compiled branch",
                ));
            }
            *graph = graph.replacen(&needle, &replacement, 1);
        }
        let crop_channels: Vec<_> = item
            .keyframes
            .iter()
            .filter(|c| c.property.starts_with("video.crop."))
            .collect();
        if crop_channels.is_empty() {
            continue;
        }
        let clip = item
            .payload
            .as_clip()
            .ok_or_else(|| invalid("crop animation requires a clip"))?;
        if !clip.video.speed.is_normal()
            || item.effects.iter().any(|e| {
                !matches!(
                    e.capability.as_str(),
                    "video.color.basic"
                        | "video.blur.gaussian"
                        | "video.sharpen"
                        | "video.vignette"
                        | "video.grain"
                )
            })
        {
            return Err(invalid(
                "animated crop requires normal-speed footage and supported linear effects",
            ));
        }
        let mut times = vec![RationalTime::zero(item.duration.rate)];
        times.extend(
            crop_channels
                .iter()
                .flat_map(|c| c.points.iter().map(|p| p.time)),
        );
        let evaluate = |side: &str, time: RationalTime, fallback: f64| -> Result<f64> {
            if let Some(channel) = crop_channels
                .iter()
                .find(|c| c.property == format!("video.crop.{side}"))
            {
                expression(channel, None, "t")?;
                Ok(agentcut_core::normalize::evaluate_number(channel, time)?)
            } else {
                Ok(fallback)
            }
        };
        let width = clip.video.crop.left + clip.video.crop.right;
        let height = clip.video.crop.top + clip.video.crop.bottom;
        for time in times {
            let left = evaluate("left", time, clip.video.crop.left)?;
            let right = evaluate("right", time, clip.video.crop.right)?;
            let top = evaluate("top", time, clip.video.crop.top)?;
            let bottom = evaluate("bottom", time, clip.video.crop.bottom)?;
            if (left + right - width).abs() > 0.000001 || (top + bottom - height).abs() > 0.000001 {
                return Err(invalid(
                    "animated crop viewport size must remain constant; pan with complementary inset channels",
                ));
            }
        }
        let asset = project.require_asset(&clip.asset_id)?;
        let size = asset
            .metadata
            .video
            .as_ref()
            .ok_or_else(|| invalid("animated crop requires video"))?
            .display_size();
        let side_expression = |side: &str, fallback: f64| -> Result<String> {
            crop_channels
                .iter()
                .find(|c| c.property == format!("video.crop.{side}"))
                .map_or(Ok(fallback.to_string()), |c| expression(c, None, "t"))
        };
        let x = side_expression("left", clip.video.crop.left)?;
        let y = side_expression("top", clip.video.crop.top)?;
        let crop = element.placement.source_crop;
        let needle = format!("crop={}:{}:{}:{}", crop.width, crop.height, crop.x, crop.y);
        let replacement = format!(
            "crop={}:{}:'({x})*{}':'({y})*{}'",
            crop.width, crop.height, size.width, size.height
        );
        // A simple crop branch ends at its unique vN producer. Never alter a
        // different clip that happens to have identical static crop numbers.
        let producer = format!("[v{ordinal}]");
        let end = graph
            .find(&producer)
            .ok_or_else(|| invalid("animated crop producer missing"))?;
        let begin = graph[..end].rfind(';').map_or(0, |i| i + 1);
        let branch = &graph[begin..end];
        if !branch.contains(&needle) {
            return Err(invalid(
                "animated crop is incompatible with this compiled branch",
            ));
        }
        let updated = branch.replacen(&needle, &replacement, 1);
        graph.replace_range(begin..end, &updated);
    }
    Ok(())
}
