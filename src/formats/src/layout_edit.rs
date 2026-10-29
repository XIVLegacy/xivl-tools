//! Bounded edits of one shared MapLayout model and collision binding.

use crate::error::{ErrorKind, FormatError, Result};
use crate::resource::ResourceId;
use crate::zone::{parse_layout_with_structure, Layout, LayoutInspection, Matrix4};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutBindingRow {
    /// Zero-based resource-table row, not a byte offset.
    pub row: u32,
    pub expected_id: ResourceId,
    pub output_id: ResourceId,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutBindingPlacement {
    pub source_node: u32,
    pub source_block: u32,
    pub world_matrix: Matrix4,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutBindingRequest {
    pub resource_key: String,
    pub model: LayoutBindingRow,
    pub collision: LayoutBindingRow,
    pub expected_placements: [LayoutBindingPlacement; 4],
}

/// Replace two row IDs after validating their selectors and four paired placements.
/// Every byte outside the parsed ID fields, including the scene, is preserved.
pub fn edit_layout_binding(template: &[u8], request: &LayoutBindingRequest) -> Result<Vec<u8>> {
    let (mut expected_layout, mut expected_inspection) = parse_layout_with_structure(template)?;
    let model_at = selected_field(
        &expected_layout,
        &expected_inspection,
        request,
        request.model,
        "brt",
    )?;
    let collision_at = selected_field(
        &expected_layout,
        &expected_inspection,
        request,
        request.collision,
        "bhp",
    )?;
    let outputs = [request.model.output_id, request.collision.output_id];
    if outputs.iter().any(|id| id.value() == 0) || outputs[0] == outputs[1] {
        return Err(FormatError::new(
            ErrorKind::InvalidResourceId,
            model_at as u64,
            "output IDs must be nonzero and distinct",
        ));
    }
    for (index, row) in expected_layout.resources.iter().enumerate() {
        if index != request.model.row as usize
            && index != request.collision.row as usize
            && outputs.contains(&row.resource_id)
        {
            return Err(FormatError::new(
                ErrorKind::InvalidResourceId,
                expected_inspection.rows[index].span.offset + 0x14,
                "output ID collides with an unmodified source row",
            ));
        }
    }
    require_placements(&expected_layout, &expected_inspection, request)?;

    let mut output = template.to_vec();
    for (selection, at) in [(request.model, model_at), (request.collision, collision_at)] {
        output[at..at + 4].copy_from_slice(&selection.output_id.value().to_le_bytes());
        expected_layout.resources[selection.row as usize].resource_id = selection.output_id;
        expected_inspection.rows[selection.row as usize].resource_id = selection.output_id;
    }
    for object in &mut expected_layout.objects {
        object.model = Some(request.model.output_id);
        object.phb = Some(request.collision.output_id);
    }
    let (actual_layout, actual_inspection) = parse_layout_with_structure(&output)?;
    if actual_layout != expected_layout || actual_inspection != expected_inspection {
        return Err(binding_error(
            expected_inspection.payload.offset,
            "edited layout changed declarations, placement identities or transforms",
        ));
    }
    if output.len() != template.len() {
        return Err(binding_error(0, "edited layout changed the input length"));
    }
    for (offset, (before, after)) in template.iter().zip(&output).enumerate() {
        if before != after
            && !(model_at..model_at + 4).contains(&offset)
            && !(collision_at..collision_at + 4).contains(&offset)
        {
            return Err(binding_error(
                offset as u64,
                "edited layout changed a byte outside the two ID fields",
            ));
        }
    }
    Ok(output)
}

fn selected_field(
    layout: &Layout,
    inspection: &LayoutInspection,
    request: &LayoutBindingRequest,
    selection: LayoutBindingRow,
    tag: &str,
) -> Result<usize> {
    let index = selection.row as usize;
    let row = layout
        .resources
        .get(index)
        .ok_or_else(|| binding_error(0x24, "binding row is outside the resource table"))?;
    let span = inspection.rows[index].span;
    if row.name != request.resource_key || row.type_tag != tag {
        return Err(binding_error(
            span.offset,
            "binding key or tag does not match the selected row",
        ));
    }
    if layout
        .resources
        .iter()
        .filter(|row| row.name == request.resource_key && row.type_tag == tag)
        .count()
        != 1
    {
        return Err(binding_error(
            span.offset,
            "binding key/tag must select exactly one row",
        ));
    }
    // MapLayout stores the little-endian resource ID at row start + 0x14.
    let offset = span.offset + 0x14;
    if row.resource_id != selection.expected_id || row.resource_id.value() == 0 {
        return Err(FormatError::new(
            ErrorKind::InvalidResourceId,
            offset,
            "selected row does not contain the expected nonzero source ID",
        ));
    }
    Ok(offset as usize)
}

fn require_placements(
    layout: &Layout,
    inspection: &LayoutInspection,
    request: &LayoutBindingRequest,
) -> Result<()> {
    if layout.objects.len() != 4 {
        return Err(binding_error(
            inspection.payload.offset,
            "binding requires exactly four paired placements",
        ));
    }
    let mut unmatched = layout.objects.iter().collect::<Vec<_>>();
    for expected in &request.expected_placements {
        let index = unmatched
            .iter()
            .position(|object| {
                object.source_node == expected.source_node
                    && object.source_block == expected.source_block
                    && object.world_matrix == expected.world_matrix
            })
            .ok_or_else(|| {
                binding_error(
                    inspection.payload.offset,
                    "expected placement identity or matrix does not match layout",
                )
            })?;
        let object = unmatched.remove(index);
        if object.model != Some(request.model.expected_id)
            || object.phb != Some(request.collision.expected_id)
            || object.resource_key.as_deref() != Some(request.resource_key.as_str())
            || !object.boxes.is_empty()
        {
            return Err(binding_error(
                inspection.payload.offset + u64::from(object.source_node),
                "binding requires paired model/PHB placements without collision boxes",
            ));
        }
    }
    Ok(())
}

fn binding_error(offset: u64, detail: &str) -> FormatError {
    FormatError::new(ErrorKind::InvalidAttributeValue, offset, detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE: &[u8] =
        include_bytes!("../../../tests/fixtures/public/zone-project/client/layout.bin");

    fn request() -> LayoutBindingRequest {
        LayoutBindingRequest {
            resource_key: "floor".into(),
            model: LayoutBindingRow {
                row: 1,
                expected_id: ResourceId::new(0x01020304),
                output_id: ResourceId::new(0xffff0001),
            },
            collision: LayoutBindingRow {
                row: 2,
                expected_id: ResourceId::new(0x05060708),
                output_id: ResourceId::new(0xffff0002),
            },
            expected_placements: [(16.0, -16.0), (16.0, 16.0), (-16.0, 16.0), (-16.0, -16.0)].map(
                |(x, z)| LayoutBindingPlacement {
                    source_node: 1008,
                    source_block: 1008,
                    world_matrix: Matrix4 {
                        values: [
                            [1.0, 0.0, 0.0, x],
                            [0.0, 1.0, 0.0, 0.0],
                            [0.0, 0.0, 1.0, z],
                            [0.0, 0.0, 0.0, 1.0],
                        ],
                    },
                },
            ),
        }
    }

    fn assert_refusal(
        bytes: &[u8],
        request: &LayoutBindingRequest,
        kind: ErrorKind,
        offset: u64,
        detail: &str,
    ) {
        let before = bytes.to_vec();
        let error = edit_layout_binding(bytes, request).unwrap_err();
        assert_eq!(error.kind(), kind);
        assert_eq!(error.offset(), offset);
        assert_eq!(error.detail(), detail);
        assert_eq!(bytes, before);
    }

    fn assert_edit(bytes: &[u8], request: &LayoutBindingRequest, model_at: usize, phb_at: usize) {
        let before = bytes.to_vec();
        let output = edit_layout_binding(bytes, request).unwrap();
        assert_eq!(bytes, before);
        assert_eq!(output.len(), bytes.len());
        assert_eq!(&output[model_at..model_at + 4], &[1, 0, 255, 255]);
        assert_eq!(&output[phb_at..phb_at + 4], &[2, 0, 255, 255]);
        // Literal field positions keep this comparison independent of parsed row spans.
        for offset in 0..bytes.len() {
            if !(model_at..model_at + 4).contains(&offset)
                && !(phb_at..phb_at + 4).contains(&offset)
            {
                assert_eq!(output[offset], bytes[offset], "byte {offset:#x}");
            }
        }
        assert_eq!(&output[0xc0..], &bytes[0xc0..]);
        let (layout, inspection) = parse_layout_with_structure(&output).unwrap();
        assert_eq!(inspection.entry_count, 4);
        assert_eq!(inspection.scene_offset, 0xc0);
        assert_eq!(layout.objects.len(), 4);
        assert!(layout.objects.iter().all(|object| {
            object.model == Some(ResourceId::new(0xffff0001))
                && object.phb == Some(ResourceId::new(0xffff0002))
                && object.resource_key.as_deref() == Some("floor")
        }));
        let original = parse_layout_with_structure(bytes).unwrap().0;
        let mut expected = original.clone();
        expected.resources[request.model.row as usize].resource_id = ResourceId::new(0xffff0001);
        expected.resources[request.collision.row as usize].resource_id =
            ResourceId::new(0xffff0002);
        for object in &mut expected.objects {
            object.model = Some(ResourceId::new(0xffff0001));
            object.phb = Some(ResourceId::new(0xffff0002));
        }
        assert_eq!(layout, expected);
        assert_eq!(edit_layout_binding(bytes, request).unwrap(), output);
    }

    #[test]
    fn layout_edit_changes_only_two_words_and_preserves_scene() {
        assert_edit(TEMPLATE, &request(), 0x74, 0x94);
    }

    #[test]
    fn layout_edit_uses_selected_rows_after_reordering() {
        let mut bytes = TEMPLATE.to_vec();
        for (destination, source) in [3, 2, 0, 1].into_iter().enumerate() {
            bytes[0x40 + destination * 32..0x60 + destination * 32]
                .copy_from_slice(&TEMPLATE[0x40 + source * 32..0x60 + source * 32]);
        }
        let mut binding = request();
        assert_refusal(
            &bytes,
            &binding,
            ErrorKind::InvalidAttributeValue,
            0x60,
            "binding key or tag does not match the selected row",
        );
        binding.model.row = 3;
        binding.collision.row = 1;
        assert_edit(&bytes, &binding, 0xb4, 0x74);
    }

    #[test]
    fn layout_edit_refuses_stale_key_rows_tags_and_ids() {
        let mut binding = request();
        binding.resource_key = "other".into();
        assert_refusal(
            TEMPLATE,
            &binding,
            ErrorKind::InvalidAttributeValue,
            0x60,
            "binding key or tag does not match the selected row",
        );
        binding = request();
        binding.model.row = u32::MAX;
        assert_refusal(
            TEMPLATE,
            &binding,
            ErrorKind::InvalidAttributeValue,
            0x24,
            "binding row is outside the resource table",
        );
        let mut bytes = TEMPLATE.to_vec();
        bytes[0x90..0x94].copy_from_slice(b"tex\0");
        assert_refusal(
            &bytes,
            &request(),
            ErrorKind::InvalidAttributeValue,
            0x80,
            "binding key or tag does not match the selected row",
        );
        for (model, at) in [(true, 0x74), (false, 0x94)] {
            binding = request();
            if model {
                binding.model.expected_id = ResourceId::new(42);
            } else {
                binding.collision.expected_id = ResourceId::new(42);
            }
            assert_refusal(
                TEMPLATE,
                &binding,
                ErrorKind::InvalidResourceId,
                at,
                "selected row does not contain the expected nonzero source ID",
            );
        }
    }

    #[test]
    fn layout_edit_refuses_duplicate_key_tag_rows() {
        for source in [0x60, 0x80] {
            let mut bytes = TEMPLATE.to_vec();
            bytes[0xa0..0xc0].copy_from_slice(&TEMPLATE[source..source + 32]);
            assert_refusal(
                &bytes,
                &request(),
                ErrorKind::InvalidAttributeValue,
                source as u64,
                "binding key/tag must select exactly one row",
            );
        }
    }

    #[test]
    fn layout_edit_refuses_zero_equal_and_colliding_output_ids() {
        for (model, collision) in [(0, 2), (1, 0), (42, 42)] {
            let mut binding = request();
            binding.model.output_id = ResourceId::new(model);
            binding.collision.output_id = ResourceId::new(collision);
            assert_refusal(
                TEMPLATE,
                &binding,
                ErrorKind::InvalidResourceId,
                0x74,
                "output IDs must be nonzero and distinct",
            );
        }
        for model in [true, false] {
            let mut binding = request();
            if model {
                binding.model.output_id = ResourceId::new(0x090a0b0c);
            } else {
                binding.collision.output_id = ResourceId::new(0x090a0b0c);
            }
            assert_refusal(
                TEMPLATE,
                &binding,
                ErrorKind::InvalidResourceId,
                0xb4,
                "output ID collides with an unmodified source row",
            );
        }
    }

    #[test]
    fn layout_edit_refuses_unexpected_or_unpaired_placements() {
        for change in 0..4 {
            let mut binding = request();
            match change {
                0 => binding.expected_placements[0].source_node += 4,
                1 => binding.expected_placements[0].source_block += 4,
                2 => binding.expected_placements[0].world_matrix.values[0][3] = 17.0,
                _ => binding.expected_placements[0] = binding.expected_placements[1],
            }
            assert_refusal(
                TEMPLATE,
                &binding,
                ErrorKind::InvalidAttributeValue,
                0xf0,
                "expected placement identity or matrix does not match layout",
            );
        }
        // Replace the last object-list entry with LaySettings, leaving three instances.
        let mut bytes = TEMPLATE.to_vec();
        bytes[0x11c..0x120].copy_from_slice(&0x170u32.to_le_bytes());
        assert_refusal(
            &bytes,
            &request(),
            ErrorKind::InvalidAttributeValue,
            0xf0,
            "binding requires exactly four paired placements",
        );

        // Redirect placements to a separate model-only key while retaining both selected rows.
        let mut bytes = TEMPLATE.to_vec();
        bytes[0xa0..0xb0].fill(0);
        bytes[0xa0..0xa4].copy_from_slice(b"tile");
        bytes[0xb0..0xb4].copy_from_slice(b"brt\0");
        bytes[0x504..0x508].copy_from_slice(&0x4f8u32.to_le_bytes());
        assert_refusal(
            &bytes,
            &request(),
            ErrorKind::InvalidAttributeValue,
            0x4e0,
            "binding requires paired model/PHB placements without collision boxes",
        );
    }

    #[test]
    fn layout_edit_propagates_bounded_parse_failure() {
        let error = edit_layout_binding(&TEMPLATE[..0x30], &request()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidAttributeValue);
        assert_eq!(error.offset(), 0);
        assert_eq!(error.detail(), "expected MapLayoutResourceData header");
    }
}
