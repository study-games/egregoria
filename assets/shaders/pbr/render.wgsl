#include "dither.wgsl"
#include "tonemap.wgsl"

fn fresnelSchlick(cosTheta: f32, F0: vec3<f32>) -> vec3<f32> {
    return F0 + (1.0 - F0) * pow(clamp(1.0 - cosTheta, 0.0, 1.0), 5.0);
}

fn fresnelSchlickRoughness(cosTheta: f32, F0: vec3<f32>, roughness: f32) -> vec3<f32> {
    return F0 + (max(vec3(1.0 - roughness), F0) - F0) * pow(clamp(1.0 - cosTheta, 0.0, 1.0), 5.0);
}

const PI: f32 = 3.14159265359;

fn DistributionGGX(NdotH: f32, roughness: f32) -> f32 {
    let a: f32      = roughness*roughness;
    let a2: f32     = a*a;
    let NdotH2: f32 = NdotH*NdotH;

    let num: f32   = a2;
    var denom: f32 = (NdotH2 * (a2 - 1.0) + 1.0);
    denom = PI * denom * denom;

    return num / denom;
}

fn GeometrySchlickGGX(NdotV: f32, roughness: f32) -> f32 {
    let r: f32 = (roughness + 1.0);
    let k: f32 = (r*r) / 8.0;

    let num: f32   = NdotV;
    let denom: f32 = NdotV * (1.0 - k) + k;

    return num / denom;
}

fn GeometrySmith(NdotV: f32, NdotL: f32, roughness: f32) -> f32 {
    let ggx2: f32  = GeometrySchlickGGX(NdotV, roughness);
    let ggx1: f32  = GeometrySchlickGGX(NdotL, roughness);

    return ggx1 * ggx2;
}

const LIGHTCHUNK_SIZE: f32 = 32.0;

fn decodeLight(chunk: vec2<u32>, light: u32) -> vec3<f32> {
    let x: u32 = light >> 20u;
    let y: u32 = (light >> 8u) & (0xFFFu);
    let z: u32 = light & 0xFFu;
    return vec3<f32>(
        f32(x) / f32(1u << 12u) * (LIGHTCHUNK_SIZE * 3.0) - LIGHTCHUNK_SIZE + f32(chunk.x) * LIGHTCHUNK_SIZE,
        f32(y) / f32(1u << 12u) * (LIGHTCHUNK_SIZE * 3.0) - LIGHTCHUNK_SIZE + f32(chunk.y) * LIGHTCHUNK_SIZE,
        f32(z) / f32(1u << 8u) * (LIGHTCHUNK_SIZE * 3.0) - LIGHTCHUNK_SIZE
    );
}

fn lightPower(wpos: vec3<f32>, light: vec3<f32>) -> f32 {
    let dist: f32 = 1.0 - min(length(wpos - light) / 32.0, 1.0);
    return dist * dist;
}

fn render(sun: vec3<f32>,
          V: vec3<f32>,
          position: vec2<f32>,
          normal: vec3<f32>,
          albedo: vec3<f32>,
          F0: vec3<f32>,
          F_spec: vec3<f32>,
          sun_col: vec3<f32>,
          irradiance_diffuse: vec3<f32>,
          specular: vec3<f32>,
          metallic: f32,
          roughness: f32,
          shadow_v: f32,
          ssao: f32,
          t_lightdata: texture_2d<u32>,
          t_lightdata2: texture_2d<u32>,
          wpos: vec3<f32>
          ) -> vec3<f32>  {
    let chunk_id: vec2<u32> = vec2<u32>(u32(wpos.x / LIGHTCHUNK_SIZE), u32(wpos.y / LIGHTCHUNK_SIZE));
    let lightdata: vec4<u32> = textureLoad(t_lightdata, chunk_id, 0);

    let H: vec3<f32> = normalize(sun + V);
    let NdotL: f32 = max(dot(normal, sun), 0.0);
    let NdotV: f32 = max(dot(normal, V), 0.0);

    let NDF: f32 = DistributionGGX(dot(normal, H), roughness);
    let G: f32   = GeometrySmith(NdotV, NdotL, roughness);
    let F: vec3<f32>  = fresnelSchlick(max(dot(H, V), 0.0), F0);

    let kS: vec3<f32> = F;
    var kD: vec3<f32> = vec3(1.0) - kS;

    kD *= 1.0 - vec3(metallic);


    let numerator: vec3<f32>      = NDF * G * F;
    let denominator: f32    = 4.0 * max(NdotV, 0.0) * NdotL  + 0.0001;
    let specular_light: vec3<f32> = numerator / denominator;

    let Lo: vec3<f32> = (kD * albedo * ssao / PI + specular_light) * (4.0 * shadow_v * sun_col) * NdotL;

    let dkS: vec3<f32> = F_spec;
    var dkD: vec3<f32> = 1.0 - dkS;
    dkD *= 1.0 - vec3(metallic);


    var mind: f32 = 0.0;
    var c: f32 = 0.0;
    if(lightdata.x != 0u) {
        mind += lightPower(wpos, decodeLight(chunk_id, lightdata.x));
        c += 1.0;
    }
    if(lightdata.y != 0u) {
        mind = mind + lightPower(wpos, decodeLight(chunk_id, lightdata.y));
        c += 1.0;
    }
    if(lightdata.z != 0u) {
        mind = mind + lightPower(wpos, decodeLight(chunk_id, lightdata.z));
        c += 1.0;
    }
    if(lightdata.w != 0u) {
        mind = mind + lightPower(wpos, decodeLight(chunk_id, lightdata.w));
        c += 1.0;
        let lightdata2: vec4<u32> = textureLoad(t_lightdata2, chunk_id, 0);
        if(lightdata2.x != 0u) {
            mind += lightPower(wpos, decodeLight(chunk_id, lightdata2.x));
            c += 1.0;
        }
        if(lightdata2.y != 0u) {
            mind = mind + lightPower(wpos, decodeLight(chunk_id, lightdata2.y));
            c += 1.0;
        }
        if(lightdata2.z != 0u) {
            mind = mind + lightPower(wpos, decodeLight(chunk_id, lightdata2.z));
            c += 1.0;
        }
        if(lightdata2.w != 0u) {
            mind = mind + lightPower(wpos, decodeLight(chunk_id, lightdata2.w));
            c += 1.0;
        }
    }

    if (c == 8.0) {
    }

    let ambient: vec3<f32> = (0.2 * dkD * (0.1 + mind * 2.0 + irradiance_diffuse) * albedo + specular) * ssao;
    var color: vec3<f32>   = ambient + Lo;

    color = tonemap(color);

    color += dither(position);


    return color;
}