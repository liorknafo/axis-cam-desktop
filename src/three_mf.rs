//! Geometry-only 3MF preview: core meshes, components, build transforms and
//! Production-extension model references. No printing/material interpretation.
use roxmltree::{Document, Node};
use std::{
    collections::{HashMap, HashSet},
    io::{Cursor, Read},
};
type Tri = [[f32; 3]; 3];
const CORE: &str = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02";
const PROD: &str = "http://schemas.microsoft.com/3dmanufacturing/production/2015/06";
const REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const MODEL_REL: &str = "http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel";
const LIMIT: usize = 200_000;
const XML_LIMIT: u64 = 100_000_000;
#[derive(Clone, Copy, Debug)]
struct Transform([f64; 12]);
impl Transform {
    const ID: Self = Self([1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.]);
    fn parse(s: Option<&str>) -> Result<Self, String> {
        let Some(s) = s else { return Ok(Self::ID) };
        let n: Vec<f64> = s
            .split_whitespace()
            .map(|s| {
                s.parse::<f64>()
                    .map_err(|_| "Invalid 3MF transform".to_string())
            })
            .collect::<Result<_, _>>()?;
        let a: [f64; 12] = n.try_into().map_err(|_| "3MF transforms need 12 numbers")?;
        let t = Self(a);
        if a.iter().any(|v| !v.is_finite()) || t.det().abs() < 1e-12 {
            return Err("Non-finite or singular 3MF transform".into());
        }
        Ok(t)
    }
    fn point(self, p: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|i| {
            p[0] * self.0[i] + p[1] * self.0[3 + i] + p[2] * self.0[6 + i] + self.0[9 + i]
        })
    }
    fn det(self) -> f64 {
        let a = self.0;
        a[0] * (a[4] * a[8] - a[5] * a[7]) - a[1] * (a[3] * a[8] - a[5] * a[6])
            + a[2] * (a[3] * a[7] - a[4] * a[6])
    }
    fn then(self, parent: Self) -> Self {
        let mut a = [0.; 12];
        for r in 0..3 {
            for c in 0..3 {
                a[r * 3 + c] = (0..3)
                    .map(|k| self.0[r * 3 + k] * parent.0[k * 3 + c])
                    .sum();
            }
        }
        a[9..12].copy_from_slice(&parent.point([self.0[9], self.0[10], self.0[11]]));
        Self(a)
    }
}
#[derive(Clone)]
struct Ref {
    path: String,
    id: u32,
    transform: Transform,
}
enum Object {
    Mesh(Vec<Tri>),
    Components(Vec<Ref>),
}
struct Model {
    unit: f64,
    objects: HashMap<u32, Object>,
    build: Vec<Ref>,
}
fn attr<'a>(n: Node<'a, '_>, key: &str) -> Result<&'a str, String> {
    n.attribute(key)
        .ok_or_else(|| format!("3MF {} missing {key}", n.tag_name().name()))
}
fn id(n: Node<'_, '_>, key: &str) -> Result<u32, String> {
    attr(n, key)?
        .parse::<u32>()
        .ok()
        .filter(|i| *i > 0)
        .ok_or_else(|| format!("Invalid 3MF {key}"))
}
fn number(n: Node<'_, '_>, key: &str) -> Result<f32, String> {
    attr(n, key)?
        .parse::<f32>()
        .ok()
        .filter(|x| x.is_finite())
        .ok_or_else(|| format!("Invalid 3MF coordinate {key}"))
}
fn children<'a, 'b>(n: Node<'a, 'b>, name: &'a str) -> impl Iterator<Item = Node<'a, 'b>> {
    n.children().filter(move |n| n.has_tag_name((CORE, name)))
}
fn child<'a, 'b>(n: Node<'a, 'b>, name: &'a str) -> Result<Node<'a, 'b>, String> {
    children(n, name)
        .next()
        .ok_or_else(|| format!("3MF missing {name}"))
}
fn xml(s: &str) -> Result<Document<'_>, String> {
    Document::parse_with_options(
        s,
        roxmltree::ParsingOptions {
            nodes_limit: 3_000_000,
            ..Default::default()
        },
    )
    .map_err(|e| format!("Invalid 3MF XML: {e}"))
}
// Package URIs never become filesystem paths and never cause network requests.
fn package_path(base: &str, target: &str) -> Result<String, String> {
    if target.contains(['\\', ':', '?', '#', '\0', '%']) {
        return Err("Unsupported 3MF package URI".into());
    }
    let joined = if target.starts_with('/') {
        target.to_string()
    } else {
        format!("{base}{target}")
    };
    let mut parts = vec![];
    for p in joined.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err("3MF reference leaves package root".into());
                }
            }
            p => parts.push(p),
        }
    }
    if parts.is_empty() {
        return Err("Empty 3MF package path".into());
    }
    Ok(parts.join("/"))
}
fn parse_ref(n: Node<'_, '_>, path: &str, root: &str) -> Result<Ref, String> {
    let target = if let Some(p) = n.attribute((PROD, "path")) {
        if path != root || !p.starts_with('/') {
            return Err(
                "3MF external component paths must originate in the root model and be absolute"
                    .into(),
            );
        }
        package_path("", p)?
    } else {
        path.to_string()
    };
    Ok(Ref {
        path: target,
        id: id(n, "objectid")?,
        transform: Transform::parse(n.attribute("transform"))?,
    })
}
fn model(s: &str, path: &str, root: &str) -> Result<Model, String> {
    let d = xml(s)?;
    let n = d.root_element();
    if !n.has_tag_name((CORE, "model")) {
        return Err("Unsupported 3MF model namespace".into());
    }
    for p in n
        .attribute("requiredextensions")
        .unwrap_or("")
        .split_whitespace()
    {
        if n.lookup_namespace_uri(Some(p)) != Some(PROD) {
            return Err(format!("Unsupported required 3MF extension: {p}"));
        }
    }
    let unit = match n.attribute("unit").unwrap_or("millimeter") {
        "micron" => 0.001,
        "millimeter" => 1.,
        "centimeter" => 10.,
        "inch" => 25.4,
        "foot" => 304.8,
        "meter" => 1000.,
        u => return Err(format!("Unknown 3MF unit: {u}")),
    };
    let mut objects = HashMap::new();
    let mut total = 0;
    for o in children(child(n, "resources")?, "object") {
        if objects.len() >= 10_000 {
            return Err("Too many 3MF objects".into());
        }
        let object = if let Some(m) = children(o, "mesh").next() {
            if children(o, "components").next().is_some() {
                return Err("3MF object contains both a mesh and components".into());
            }
            let mut vertices = vec![];
            for v in children(child(m, "vertices")?, "vertex") {
                if vertices.len() >= 600_000 {
                    return Err("Too many 3MF vertices".into());
                }
                vertices.push([number(v, "x")?, number(v, "y")?, number(v, "z")?]);
            }
            let mut triangles = vec![];
            for t in children(child(m, "triangles")?, "triangle") {
                total += 1;
                if total > LIMIT {
                    return Err("3MF exceeds the 200,000-triangle preview limit".into());
                }
                let mut tri = [[0.; 3]; 3];
                for (i, k) in ["v1", "v2", "v3"].iter().enumerate() {
                    let index = attr(t, k)?
                        .parse::<usize>()
                        .map_err(|_| "Invalid 3MF triangle index")?;
                    tri[i] = *vertices
                        .get(index)
                        .ok_or("3MF triangle references a missing vertex")?;
                }
                triangles.push(tri);
            }
            Object::Mesh(triangles)
        } else {
            let mut refs = vec![];
            for c in children(child(o, "components")?, "component") {
                refs.push(parse_ref(c, path, root)?);
                if refs.len() > 10_000 {
                    return Err("Too many 3MF components".into());
                }
            }
            if refs.is_empty() {
                return Err("Empty 3MF component object".into());
            }
            Object::Components(refs)
        };
        if objects.insert(id(o, "id")?, object).is_some() {
            return Err("Duplicate 3MF object id".into());
        }
    }
    let mut build = vec![];
    if path == root {
        for item in children(child(n, "build")?, "item") {
            // Printable=false items are intentionally excluded from the build preview.
            match item.attribute("printable").unwrap_or("true") {
                "false" | "0" => continue,
                "true" | "1" => {}
                _ => return Err("Invalid 3MF printable value".into()),
            }
            build.push(parse_ref(item, path, root)?);
            if build.len() > 10_000 {
                return Err("Too many 3MF build items".into());
            }
        }
    }
    Ok(Model {
        unit,
        objects,
        build,
    })
}
pub fn load(bytes: &[u8]) -> Result<Vec<Tri>, String> {
    read(bytes, XML_LIMIT)
}
// `limit` caps the XML read from the package, counted as it inflates: an entry's
// declared size is the archive author's claim and bounds nothing.
fn read(bytes: &[u8], limit: u64) -> Result<Vec<Tri>, String> {
    let mut zip =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("Invalid 3MF ZIP: {e}"))?;
    if zip.len() > 4096 {
        return Err("3MF has too many package entries".into());
    }
    let mut entries = HashMap::new();
    for i in 0..zip.len() {
        let f = zip.by_index_raw(i).map_err(|e| e.to_string())?;
        if f.is_dir() {
            continue;
        }
        if entries.insert(package_path("", f.name())?, i).is_some() {
            return Err("Duplicate 3MF package entry".into());
        }
    }
    // Only the parts the root relationship reaches are read, each only as far as the limit leaves.
    let mut total = 0u64;
    let mut part = |name: &str, missing: &str| -> Result<String, String> {
        let mut s = vec![];
        zip.by_index(*entries.get(name).ok_or(missing)?)
            .map_err(|e| e.to_string())?
            .take(limit + 1 - total)
            .read_to_end(&mut s)
            .map_err(|e| format!("Cannot read 3MF XML: {e}"))?;
        total += s.len() as u64;
        if total > limit {
            return Err("3MF expanded XML exceeds 100 MB".into());
        }
        String::from_utf8(s).map_err(|_| "3MF XML is not UTF-8".into())
    };
    let rels = part("_rels/.rels", "3MF is missing package relationships")?;
    let rels = xml(&rels)?;
    let roots: Vec<_> = rels
        .descendants()
        .filter(|n| n.has_tag_name((REL, "Relationship")) && n.attribute("Type") == Some(MODEL_REL))
        .collect();
    if roots.len() != 1 {
        return Err("3MF must have exactly one root model relationship".into());
    }
    if roots[0].attribute("TargetMode") == Some("External") {
        return Err("External 3MF URLs are unsupported".into());
    }
    let root = package_path("", attr(roots[0], "Target")?)?;
    let root_model = model(&part(&root, "3MF root model is missing")?, &root, &root)?;
    let build = root_model.build.clone();
    let mut models = HashMap::from([(root.clone(), root_model)]);
    let mut pending = vec![root.clone()];
    while let Some(path) = pending.pop() {
        let m = &models[&path];
        let mut refs = m.build.clone();
        for o in m.objects.values() {
            if let Object::Components(cs) = o {
                refs.extend(cs.iter().cloned())
            }
        }
        for r in refs {
            if !models.contains_key(&r.path) {
                if models.len() >= 128 {
                    return Err("Too many 3MF model parts".into());
                }
                let m = model(
                    &part(&r.path, "Referenced 3MF model part is missing")?,
                    &r.path,
                    &root,
                )?;
                // Different model-unit frames are uncommon; reject instead of guessing how translations should scale.
                if m.unit != models[&root].unit {
                    return Err("Mixed units across 3MF model parts are not supported yet".into());
                }
                models.insert(r.path.clone(), m);
                pending.push(r.path);
            }
        }
    }
    let mut out = vec![];
    let mut active = HashSet::new();
    let mut visits = 0;
    fn expand(
        r: &Ref,
        parent: Transform,
        models: &HashMap<String, Model>,
        active: &mut HashSet<(String, u32)>,
        out: &mut Vec<Tri>,
        visits: &mut usize,
    ) -> Result<(), String> {
        *visits += 1;
        if *visits > 100_000 || active.len() >= 64 {
            return Err("3MF component nesting/instance limit exceeded".into());
        }
        let key = (r.path.clone(), r.id);
        if !active.insert(key.clone()) {
            return Err("Cyclic 3MF component reference".into());
        }
        let m = models.get(&r.path).ok_or("Missing 3MF model")?;
        let t = r.transform.then(parent);
        match m
            .objects
            .get(&r.id)
            .ok_or("3MF references a missing object")?
        {
            Object::Mesh(tris) => {
                if out.len() + tris.len() > LIMIT {
                    return Err("Expanded 3MF exceeds the 200,000-triangle preview limit".into());
                }
                for tri in tris {
                    let mut a = tri.map(|p| t.point(p.map(f64::from)).map(|x| (x * m.unit) as f32));
                    if a.iter().flatten().any(|x| !x.is_finite()) {
                        return Err("3MF transform overflow".into());
                    }
                    if t.det() < 0. {
                        a.swap(1, 2)
                    }
                    out.push(a);
                }
            }
            Object::Components(cs) => {
                for c in cs {
                    expand(c, t, models, active, out, visits)?
                }
            }
        }
        active.remove(&key);
        Ok(())
    }
    for r in &build {
        expand(
            r,
            Transform::ID,
            &models,
            &mut active,
            &mut out,
            &mut visits,
        )?
    }
    if out.is_empty() {
        return Err("3MF build contains no printable mesh triangles".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn archive(model: &str, extra: Option<(&str, &str)>) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(vec![]));
        let opt = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        w.start_file("_rels/.rels", opt).unwrap();
        write!(w,"<Relationships xmlns='{REL}'><Relationship Id='r' Type='{MODEL_REL}' Target='/3D/custom.model'/></Relationships>").unwrap();
        w.start_file("3D/custom.model", opt).unwrap();
        w.write_all(model.as_bytes()).unwrap();
        if let Some((p, s)) = extra {
            w.start_file(p, opt).unwrap();
            w.write_all(s.as_bytes()).unwrap()
        }
        w.finish().unwrap().into_inner()
    }
    const MESH: &str = "<object id='1'><mesh><vertices><vertex x='0' y='0' z='0'/><vertex x='1' y='0' z='0'/><vertex x='0' y='1' z='0'/></vertices><triangles><triangle v1='0' v2='1' v3='2'/></triangles></mesh></object>";
    fn doc(resources: &str, build: &str, attrs: &str) -> String {
        format!(
            "<model xmlns='{CORE}' xmlns:p='{PROD}' {attrs}><resources>{resources}</resources><build>{build}</build></model>"
        )
    }
    #[test]
    fn units_instances_and_unreferenced_resources() {
        let d = doc(
            MESH,
            "<item objectid='1' transform='1 0 0 0 1 0 0 0 1 2 0 0'/><item objectid='1'/>",
            "unit='inch'",
        );
        let t = load(&archive(&d, None)).unwrap();
        assert_eq!(t.len(), 2);
        assert!((t[0][0][0] - 50.8).abs() < 1e-4);
        assert!((t[0][1][0] - 76.2).abs() < 1e-4);
        assert!(load(&archive(&doc(MESH, "", ""), None)).is_err());
    }
    #[test]
    fn nested_rotation_translation_and_mirror() {
        let rs = format!(
            "{MESH}<object id='2'><components><component objectid='1' transform='1 0 0 0 1 0 0 0 1 2 0 0'/></components></object>"
        );
        let d = doc(
            &rs,
            "<item objectid='2' transform='0 1 0 -1 0 0 0 0 1 10 0 0'/>",
            "",
        );
        let t = load(&archive(&d, None)).unwrap();
        assert_eq!(t[0][0], [10., 2., 0.]);
        assert_eq!(t[0][1], [10., 3., 0.]);
        let d = doc(
            MESH,
            "<item objectid='1' transform='-1 0 0 0 1 0 0 0 1 0 0 0'/>",
            "",
        );
        let t = load(&archive(&d, None)).unwrap();
        assert_eq!(t[0][1], [0., 1., 0.]);
        assert_eq!(t[0][2], [-1., 0., 0.]);
    }
    #[test]
    fn production_external_part() {
        let root = doc(
            "",
            "<item objectid='1' p:path='/3D/part.model'/>",
            "requiredextensions='p'",
        );
        let part = doc(MESH, "<item objectid='999'/>", "");
        assert_eq!(
            load(&archive(&root, Some(("3D/part.model", &part))))
                .unwrap()
                .len(),
            1
        );
    }
    #[test]
    fn rejects_bad_references_indices_extensions_and_cycles() {
        for d in [
            doc(MESH, "<item objectid='99'/>", ""),
            doc(
                &MESH.replace("v3='2'", "v3='99'"),
                "<item objectid='1'/>",
                "",
            ),
            doc(
                MESH,
                "<item objectid='1'/>",
                "xmlns:x='urn:unsupported' requiredextensions='x'",
            ),
            doc(
                "<object id='1'><components><component objectid='1'/></components></object>",
                "<item objectid='1'/>",
                "",
            ),
        ] {
            assert!(load(&archive(&d, None)).is_err());
        }
        assert!(load(b"not a zip").is_err());
        assert!(Transform::parse(Some("NaN 0 0 0 1 0 0 0 1 0 0 0")).is_err());
        assert!(package_path("", "../../x").is_err());
    }

    // The archive's declared sizes are the author's claim; the limit must count what inflates.
    fn declaring_one_byte(mut z: Vec<u8>) -> Vec<u8> {
        let u32_at =
            |b: &[u8], i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap()) as usize;
        let u16_at =
            |b: &[u8], i: usize| u16::from_le_bytes(b[i..i + 2].try_into().unwrap()) as usize;
        let end = z.len() - 22;
        assert_eq!(&z[end..end + 4], b"PK\x05\x06");
        let mut cd = u32_at(&z, end + 16);
        for _ in 0..u16_at(&z, end + 10) {
            assert_eq!(&z[cd..cd + 4], b"PK\x01\x02");
            let local = u32_at(&z, cd + 42);
            assert_eq!(&z[local..local + 4], b"PK\x03\x04");
            z[local + 22..local + 26].copy_from_slice(&1u32.to_le_bytes());
            z[cd + 24..cd + 28].copy_from_slice(&1u32.to_le_bytes());
            cd += 46 + u16_at(&z, cd + 28) + u16_at(&z, cd + 30) + u16_at(&z, cd + 32);
        }
        z
    }
    #[test]
    fn inflated_parts_count_as_read_not_as_declared() {
        let pad = " ".repeat(400_000);
        let root = doc(
            "",
            &format!(
                "<item objectid='1' p:path='/3D/a.model'/><item objectid='1' p:path='/3D/b.model'/>{pad}"
            ),
            "requiredextensions='p'",
        );
        let part = doc(&format!("{MESH}{pad}"), "<item objectid='1'/>", "");
        let mut w = zip::ZipWriter::new(Cursor::new(vec![]));
        let opt = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in [
            (
                "_rels/.rels",
                format!(
                    "<Relationships xmlns='{REL}'><Relationship Id='r' Type='{MODEL_REL}' Target='/3D/custom.model'/></Relationships>"
                ),
            ),
            ("3D/custom.model", root),
            ("3D/a.model", part.clone()),
            ("3D/b.model", part),
        ] {
            w.start_file(name, opt).unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        let z = declaring_one_byte(w.finish().unwrap().into_inner());
        println!("archive bytes: {}", z.len());
        assert!(z.len() < 20_000);
        // The archive is valid: with room for its 1.2 MB it loads.
        assert_eq!(read(&z, 2_000_000).unwrap().len(), 2);
        // Over the limit it is refused once the bytes read pass it, whatever the entries declare.
        assert_eq!(
            read(&z, 1_000_000).unwrap_err(),
            "3MF expanded XML exceeds 100 MB"
        );
    }
}
