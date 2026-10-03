//! Versioned, executable color decisions. Original media is never rewritten.
use crate::{Error, Result};
use agentcut_core::Asset;
use serde_json::{Value, json};

pub const PROBE: &str = "avw.ingest.v1";
pub const HDR_FILTER: &str = "zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=mobius:param=0.3:desat=2:peak=10,zscale=t=bt709:m=bt709:r=limited:dither=error_diffusion,format=yuv420p";

pub fn capability(asset: &Asset) -> Value {
    let dovi = asset
        .extensions
        .get(PROBE)
        .and_then(|p| p["streams"].as_array())
        .and_then(|streams| {
            streams
                .iter()
                .filter_map(|s| s["side_data_list"].as_array())
                .flatten()
                .find(|s| {
                    s["side_data_type"]
                        .as_str()
                        .is_some_and(|v| v.to_lowercase().contains("dovi"))
                })
        });
    // Profile 8 with an HDR10 or HLG compatible base layer can use that layer.
    // Profile 5 has a different color representation and cannot use this path.
    let dovi_supported = dovi.is_none_or(|d| {
        d["dv_profile"] == 8 && matches!(d["dv_bl_signal_compatibility_id"].as_u64(), Some(1 | 4))
    });
    let video = asset.metadata.video.as_ref();
    let transfer = video.and_then(|v| v.color_transfer.as_deref());
    let hdr = matches!(transfer, Some("smpte2084" | "arib-std-b67"));
    let wide = video
        .and_then(|v| v.color_primaries.as_deref())
        .is_some_and(|v| v.starts_with("bt2020"));
    let supported = dovi_supported && (!wide || hdr);
    json!({"readable":true,"deliverySupported":supported,"output":"Rec.709 SDR",
        "policyVersion":1,"hdr":hdr,"dolbyVision":dovi,"usesCompatibleBaseLayer":dovi.is_some() && dovi_supported,
        "conversion":if hdr {Some(HDR_FILTER)} else {None},"referenceWhiteNits":100,"inputPeakNits":if hdr {Some(1000)} else {None},
        "unsupportedReason":if !dovi_supported {Some("Dolby Vision requires profile 8 with HDR10/HLG compatible base layer (compatibility 1 or 4)")} else if wide && !hdr {Some("BT.2020 without PQ/HLG requires an explicit qualified color transform")} else {None},
        "originalPreserved":true,"sourceClock":"seconds relative to container start; initial audio gaps are padded with silence"})
}

pub fn filter(asset: &Asset) -> Result<Option<&'static str>> {
    let report = capability(asset);
    if report["deliverySupported"] != true {
        return Err(Error::Invalid(format!(
            "asset {}: {}",
            asset.id, report["unsupportedReason"]
        )));
    }
    Ok((report["hdr"] == true).then_some(HDR_FILTER))
}

pub fn adapt(
    plan: &mut agentcut_render::compile::RenderPlan,
    project: &agentcut_core::Project,
) -> Result<Value> {
    let position = plan
        .args
        .iter()
        .position(|a| a == "-filter_complex")
        .ok_or_else(|| Error::Invalid("render graph missing".into()))?
        + 1;
    let graph = plan
        .args
        .get_mut(position)
        .ok_or_else(|| Error::Invalid("render graph missing".into()))?;
    let mut decisions = Vec::new();
    for (index, input) in plan.inputs.iter().enumerate() {
        let asset = project.require_asset(&input.asset_id)?;
        let label = format!("[{}:v]", index + 1);
        if graph.contains(&label) {
            if let Some(filter) = filter(asset)? {
                // Each source branch is converted before compositing, rather than
                // tone mapping the already mixed SDR captions and overlays.
                *graph = graph.replace(&label, &format!("{label}{filter},"));
            }
            decisions.push(json!({"assetId":asset.id,"sourceSha256":asset.fingerprint.sha256,"color":capability(asset)}));
        }
        let audio = format!("[{}:a]", index + 1);
        *graph = graph.replace(&audio, &format!("{audio}aresample=async=1:first_pts=0,"));
    }
    Ok(json!(decisions))
}
