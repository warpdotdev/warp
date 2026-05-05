struct Uniforms {
    viewport_size: vec2<f32>,
    // Padding necessary to ensure that the uniforms is 16 bytes. Some wgpu-supported devices (such as webgl) require
    // buffer bindings to be a multiple of 16 bytes.
    padding: vec2<f32>
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

@group(1) @binding(0) var imageTexture: texture_2d<f32>;
@group(1) @binding(1) var imageSampler: sampler;

struct ImageVertexShaderInput {
    // The position of the vertex in normalized device coordinates.
    @location(0) vertex_position: vec2<f32>,
    @location(1) bounds: vec4<f32>,
    @location(2) color: vec4<f32>,
    // This field is treated as a boolean to indicate how to interpret the preceding `color` field.
    // Icons allow overriding their foreground color, so for icons the whole `color` struct is used.
    // For images, only the opacity can be set, and so only the alpha channel would be used.
    @location(3) is_icon: u32,
    // Corner radius in the order top_left, top_right, bottom_left, bottom_right.
    @location(4) corner_radius: vec4<f32>,
}

struct ImageVertexShaderOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) rect_center: vec2<f32>,
    @location(1) rect_corner: vec2<f32>,
    @location(2) texture_coordinate: vec2<f32>,
    @location(4) color: vec4<f32>,
    @location(5) is_icon: u32,
    @location(6) corner_radius: vec4<f32>,
}

@vertex
fn vs_main(
    image: ImageVertexShaderInput,
) -> ImageVertexShaderOutput {
    var out: ImageVertexShaderOutput;
    var origin: vec2<f32> = image.bounds.xy;
    var size: vec2<f32> = image.bounds.zw;
    var pixel_pos: vec2<f32> = image.vertex_position * size + origin;

    // Convert the position of the item from screen coordinates into normalized device coordinates
    var device_pos: vec2<f32> = pixel_pos / uniforms.viewport_size * vec2(2.0, -2.0) + vec2(-1.0, 1.0);
    out.position = vec4<f32>(device_pos, 0.0, 1.0);

    // Re-compute size and origin such that they are clipped by the viewport bounds.
    var clipped_origin = max(origin, vec2f(0.0, 0.0));
    var clipped_size = max(min(origin + size, uniforms.viewport_size) - clipped_origin, vec2f(0.0, 0.0));
    out.rect_corner = clipped_size / 2.0;
    out.rect_center = clipped_origin + out.rect_corner;

    out.texture_coordinate = image.vertex_position;
    out.color = image.color;
    out.is_icon = image.is_icon;
    out.corner_radius = image.corner_radius;
    return out;
}

fn distance_from_rect(pixel_pos: vec2<f32>, rect_center: vec2<f32>, rect_corner: vec2<f32>, corner_radius: f32) -> f32 {
    var p: vec2<f32> = pixel_pos - rect_center;
    var q: vec2<f32> = abs(p) - rect_corner + corner_radius;
    return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - corner_radius;
}

@fragment
fn fs_main(in: ImageVertexShaderOutput) -> @location(0) vec4<f32> {
    // Sample the texture to obtain a color.
    var color_sample: vec4<f32> = textureSample(imageTexture, imageSampler, in.texture_coordinate);

    var color: vec4<f32>;
    if in.is_icon == 0u {
        // For an image, use the image color and just adjust opacity.
        color = color_sample;
        color.a *= in.color.a;
    } else {
        // There's a naga bug with wgsl --> hlsl conversion where images are always rendered as red.
        // We workaround this by first creating an intermediate color where the alpha channel is actually the
        // red channel from `color_sample` and then multiplying that by the desired opacity.
        var new_color: vec4<f32> = vec4(color_sample.r, color_sample.g, color_sample.b, color_sample.r);
        new_color.a *= in.color.a;
        // For an icon, use the specified input color.
        color = vec4(in.color.r, in.color.g, in.color.b, new_color.a);
    }

    var outer_corner_radius: f32;

    if in.position.y >= in.rect_center.y {
        // Bottom half
        if in.position.x >= in.rect_center.x {
            // Bottom right quadrant
            outer_corner_radius = in.corner_radius.w;
        } else {
            // Bottom left quadrant
            outer_corner_radius = in.corner_radius.z;
        }
    } else {
        // Top half
        if in.position.x >= in.rect_center.x {
            // Top right quadrant
            outer_corner_radius = in.corner_radius.y;
        } else {
            // Top left quadrant
            outer_corner_radius = in.corner_radius.x;
        }
    }

    var outer_distance: f32 = distance_from_rect(in.position.xy, in.rect_center, in.rect_corner, outer_corner_radius);

    // If there's a corner radius we need to do some anti aliasing to smooth out the rounded corner effect.
    if outer_corner_radius > 0. {
        color.a *= 1.0 - saturate(outer_distance + 0.5);
    }

    return color;
}
