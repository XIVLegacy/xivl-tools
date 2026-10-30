//! PNG encoding and lossless DDS repacking into canonical GTEX containers.

use std::fs;
use std::io::Write;

use serde_json::{json, Value};
use xivl_formats::dds::{self, DdsPixelFormat};
use xivl_formats::digest::sha256_hex;
use xivl_formats::gtex_pwib::{
    self, GtexResource, TaggedResource, TaggedResourceKind, TextureKind,
};
use xivl_formats::gtex_write::{build_gtex, rebuild_gtex, GtexBuild};
use xivl_formats::texture_encode::{encode_surface, mip_chain_rgba};
use xivl_formats::texture_preview::{decode_png_rgba, decode_surface};
use xivl_formats::{to_canonical_json, FormatError};

use crate::{read_capped, Failure};

const USAGE: &str = "usage: xivl encode-gtex <input.png|input.dds> --output <new-file> [--like <existing.gtex>] [--format a8r8g8b8|a4r4g4b4|dxt1|dxt5] [--mips <n>]";

struct Options<'a> {
    input: &'a str,
    output: &'a str,
    like: Option<&'a str>,
    format: Option<u8>,
    mips: Option<u8>,
}

impl<'a> Options<'a> {
    fn parse(arguments: &'a [String]) -> Result<Self, Failure> {
        let Some((input, rest)) = arguments.split_first() else {
            return Err(Failure::usage(USAGE));
        };
        let mut output = None;
        let mut like = None;
        let mut format = None;
        let mut mips = None;
        let (pairs, remainder) = rest.as_chunks::<2>();
        if !remainder.is_empty() {
            return Err(Failure::usage(USAGE));
        }
        for [flag, value] in pairs {
            let duplicate =
                match flag.as_str() {
                    "--output" => output.replace(value.as_str()).is_some(),
                    "--like" => like.replace(value.as_str()).is_some(),
                    "--format" => {
                        let index = match value.as_str() {
                            "a8r8g8b8" => 4,
                            "a4r4g4b4" => 22,
                            "dxt1" => 24,
                            "dxt5" => 26,
                            _ => return Err(Failure::usage(
                                "unknown GTEX format; expected a8r8g8b8, a4r4g4b4, dxt1 or dxt5",
                            )),
                        };
                        format.replace(index).is_some()
                    }
                    "--mips" => mips
                        .replace(value.parse::<u8>().map_err(|_| {
                            Failure::usage("--mips requires an unsigned 8-bit integer")
                        })?)
                        .is_some(),
                    _ => {
                        return Err(Failure::usage(format!(
                            "unknown encode-gtex option '{flag}'"
                        )))
                    }
                };
            if duplicate {
                return Err(Failure::usage(format!(
                    "{flag} was supplied more than once"
                )));
            }
        }
        if like.is_some() && (format.is_some() || mips.is_some()) {
            return Err(Failure::usage(
                "--like cannot be combined with --format or --mips",
            ));
        }
        if like.is_none() && format.is_none() {
            return Err(Failure::usage("--format is required without --like"));
        }
        Ok(Self {
            input,
            output: output.ok_or_else(|| Failure::usage("--output is required"))?,
            like,
            format,
            mips,
        })
    }
}

pub fn run(arguments: &[String]) -> Result<(), Failure> {
    let report = encode(Options::parse(arguments)?)?;
    std::io::stdout()
        .write_all(to_canonical_json(&report).as_bytes())
        .map_err(|error| Failure::usage(format!("cannot write stdout: {error}")))
}

fn encode(options: Options<'_>) -> Result<Value, Failure> {
    let input = read_capped(options.input)?;
    let template = options
        .like
        .map(|path| {
            let bytes = read_capped(path)?;
            let gtex = parse_gtex(&bytes)?;
            if gtex.texture_kind != TextureKind::Texture2d || gtex.flags != 0 || gtex.depth != 1 {
                return Err(Failure::usage(
                    "--like requires a 2D GTEX template with depth one",
                ));
            }
            Ok(gtex)
        })
        .transpose()?;
    let format = template
        .as_ref()
        .map(|gtex| gtex.format_index)
        .or(options.format)
        .ok_or_else(|| Failure::usage("--format is required without --like"))?;
    let name = format_name(format)?;
    let requested_mips = template
        .as_ref()
        .map(|gtex| gtex.mip_levels)
        .or(options.mips);
    let (kind, width, height, mips, surfaces, source) = if input.starts_with(b"\x89PNG\r\n\x1a\n") {
        let image = decode_png_rgba(&input).map_err(format_failure)?;
        check_dimensions(image.width, image.height, template.as_ref())?;
        let mips = requested_mips.unwrap_or(1);
        let chain =
            mip_chain_rgba(image.width, image.height, &image.rgba, mips).map_err(format_failure)?;
        let surfaces = chain
            .iter()
            .map(|(width, height, rgba)| {
                encode_surface(format, *width, *height, rgba).map_err(format_failure)
            })
            .collect::<Result<Vec<_>, _>>()?;
        ("png", image.width, image.height, mips, surfaces, chain)
    } else if input.starts_with(dds::DDS_MAGIC) {
        let image = dds::parse(&input).map_err(format_failure)?;
        check_dimensions(image.width, image.height, template.as_ref())?;
        let source_format = match image.format {
            DdsPixelFormat::A8R8G8B8 => 4,
            DdsPixelFormat::A4R4G4B4 => 22,
            DdsPixelFormat::Dxt1 => 24,
            DdsPixelFormat::Dxt5 => 26,
        };
        if source_format != format {
            return Err(Failure::usage(format!(
                "DDS format {} differs from target {name}; lossless repacking cannot transcode",
                format_name(source_format)?
            )));
        }
        let mips = requested_mips.unwrap_or(image.mip_levels);
        if mips > image.mip_levels {
            return Err(Failure::usage(format!(
                "requested {mips} mips but DDS contains {}; lossless repacking cannot generate levels",
                image.mip_levels
            )));
        }
        let surfaces = image
            .mips
            .iter()
            .take(usize::from(mips))
            .map(|mip| {
                let start = mip.span.offset as usize;
                input[start..start + mip.span.length as usize].to_vec()
            })
            .collect();
        ("dds", image.width, image.height, mips, surfaces, Vec::new())
    } else {
        return Err(Failure::usage(
            "unknown input kind; expected PNG signature or DDS magic",
        ));
    };
    let build = GtexBuild {
        format_index: format,
        width: width as u16,
        height: height as u16,
        mips: surfaces,
    };
    let bytes = build_gtex(&build).map_err(format_failure)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(options.output)
        .map_err(|error| Failure::usage(format!("cannot create output: {error}")))?;
    let written = file.write_all(&bytes).and_then(|()| file.flush());
    drop(file);
    let verified = written
        .map_err(|error| Failure::usage(format!("cannot write output: {error}")))
        .and_then(|()| {
            let output = read_capped(options.output)?;
            let gtex = parse_gtex(&output)?;
            if (gtex.format_index, gtex.width, gtex.height, gtex.mip_levels)
                != (format, build.width, build.height, mips)
                || output != bytes
                || rebuild_gtex(&output, &gtex).map_err(format_failure)? != output
            {
                return Err(Failure::parse(
                    "written GTEX failed metadata or exact rebuild verification",
                ));
            }
            let mut report = json!({
                "operation": "encode-gtex",
                "inputKind": kind,
                "format": name,
                "formatIndex": format,
                "width": width,
                "height": height,
                "mips": mips,
                "outputLength": output.len(),
                "outputSha256": sha256_hex(&output),
            });
            if kind == "png" && format != 4 {
                let mut errors = Vec::new();
                for ((width, height, rgba), surface) in source.iter().zip(&gtex.surfaces) {
                    let start = surface.source_span.offset as usize;
                    let encoded = &output[start..start + surface.source_span.length as usize];
                    let mut decoded = vec![0; rgba.len()];
                    decode_surface(format, *width, *height, encoded, &mut decoded)
                        .map_err(format_failure)?;
                    let squared_error: u64 = rgba
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .zip(decoded.as_chunks::<4>().0)
                        .map(|(a, b)| {
                            (0..3)
                                .map(|channel| {
                                    let difference = i64::from(a[channel]) - i64::from(b[channel]);
                                    (difference * difference) as u64
                                })
                                .sum::<u64>()
                        })
                        .sum();
                    errors.push(squared_error as f64 / (rgba.len() / 4 * 3) as f64);
                }
                report["rgbMsePerMip"] = json!(errors);
            }
            Ok(report)
        });
    if let Err(failure) = verified {
        fs::remove_file(options.output).map_err(|error| Failure {
            message: format!("{}; cannot remove failed output: {error}", failure.message),
            code: failure.code,
        })?;
        return Err(failure);
    }
    verified
}

fn check_dimensions(
    width: u32,
    height: u32,
    template: Option<&GtexResource>,
) -> Result<(), Failure> {
    if width > u32::from(u16::MAX) || height > u32::from(u16::MAX) {
        return Err(Failure::usage(
            "input dimensions exceed GTEX 16-bit dimensions",
        ));
    }
    if let Some(template) = template {
        if (width, height) != (u32::from(template.width), u32::from(template.height)) {
            return Err(Failure::usage(format!(
                "input dimensions {width}x{height} differ from template {}x{}; resampling is not supported",
                template.width, template.height
            )));
        }
    }
    Ok(())
}

fn parse_gtex(bytes: &[u8]) -> Result<GtexResource, Failure> {
    match gtex_pwib::parse(bytes, TaggedResourceKind::Gtex).map_err(format_failure)? {
        TaggedResource::Gtex(gtex) => Ok(gtex),
        TaggedResource::Pwib(_) => Err(Failure::parse("expected standalone GTEX")),
    }
}

fn format_name(index: u8) -> Result<&'static str, Failure> {
    match index {
        4 => Ok("a8r8g8b8"),
        22 => Ok("a4r4g4b4"),
        24 => Ok("dxt1"),
        26 => Ok("dxt5"),
        _ => Err(Failure::usage(format!(
            "unmapped target GTEX format index {index}"
        ))),
    }
}

fn format_failure(error: FormatError) -> Failure {
    Failure::parse(error.to_string())
}
