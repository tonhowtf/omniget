//! Escritor mínimo de PDF a partir de JPEGs (uma imagem por página, filtro
//! DCTDecode, sem recomprimir). É o `img2pdf` dos scripts de SlideShare
//! (estudo 50) sem dependência nova.

use anyhow::anyhow;

pub struct JpegInfo {
    pub width: u32,
    pub height: u32,
    pub components: u8,
}

/// Lê largura/altura/canais do marcador SOF do JPEG.
pub fn jpeg_info(data: &[u8]) -> anyhow::Result<JpegInfo> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return Err(anyhow!("nao e um JPEG"));
    }
    let mut i = 2usize;
    while i + 9 < data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        // marcadores sem payload
        if (0xD0..=0xD9).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        let is_sof = matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if is_sof {
            let height = u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32;
            let width = u16::from_be_bytes([data[i + 7], data[i + 8]]) as u32;
            let components = data[i + 9];
            return Ok(JpegInfo {
                width,
                height,
                components,
            });
        }
        i += 2 + len;
    }
    Err(anyhow!("JPEG sem cabecalho SOF"))
}

pub fn is_jpeg(data: &[u8]) -> bool {
    data.len() > 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF
}

/// Monta o PDF em memória. Página no tamanho da imagem em pontos (72 dpi).
pub fn build_pdf(images: &[Vec<u8>]) -> anyhow::Result<Vec<u8>> {
    if images.is_empty() {
        return Err(anyhow!("nenhuma imagem"));
    }
    let mut out: Vec<u8> = Vec::new();
    let mut offsets: Vec<usize> = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");

    // objetos: 1 catalog, 2 pages, depois por imagem: page, xobject, content
    let n_pages = images.len();
    let obj_page = |i: usize| 3 + i * 3;
    let obj_img = |i: usize| 4 + i * 3;
    let obj_content = |i: usize| 5 + i * 3;
    let total_objs = 2 + n_pages * 3;

    let push_obj = |out: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(body);
    };

    push_obj(
        &mut out,
        &mut offsets,
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
    );
    let kids: Vec<String> = (0..n_pages)
        .map(|i| format!("{} 0 R", obj_page(i)))
        .collect();
    push_obj(
        &mut out,
        &mut offsets,
        format!(
            "2 0 obj\n<< /Type /Pages /Kids [{}] /Count {} >>\nendobj\n",
            kids.join(" "),
            n_pages
        )
        .as_bytes(),
    );
    for (i, img) in images.iter().enumerate() {
        let info = jpeg_info(img)?;
        let cs = match info.components {
            1 => "/DeviceGray",
            4 => "/DeviceCMYK",
            _ => "/DeviceRGB",
        };
        let (w, h) = (info.width.max(1), info.height.max(1));
        push_obj(
            &mut out,
            &mut offsets,
            format!(
                "{} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /XObject << /Im0 {} 0 R >> >> /Contents {} 0 R >>\nendobj\n",
                obj_page(i),
                w,
                h,
                obj_img(i),
                obj_content(i)
            )
            .as_bytes(),
        );
        let mut xobj = format!(
            "{} 0 obj\n<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {} /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
            obj_img(i),
            w,
            h,
            cs,
            img.len()
        )
        .into_bytes();
        xobj.extend_from_slice(img);
        xobj.extend_from_slice(b"\nendstream\nendobj\n");
        push_obj(&mut out, &mut offsets, &xobj);
        let content = format!("q {} 0 0 {} 0 0 cm /Im0 Do Q", w, h);
        push_obj(
            &mut out,
            &mut offsets,
            format!(
                "{} 0 obj\n<< /Length {} >>\nstream\n{}\nendstream\nendobj\n",
                obj_content(i),
                content.len(),
                content
            )
            .as_bytes(),
        );
    }
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", total_objs + 1).as_bytes());
    for off in &offsets {
        out.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            total_objs + 1,
            xref_at
        )
        .as_bytes(),
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// JPEG 1x1 mínimo (cinza) só com os marcadores que o parser precisa.
    fn tiny_jpeg() -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8];
        v.extend_from_slice(&[
            0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x02, 0x00, 0x03, 0x01, 0x01, 0x11, 0x00,
        ]);
        v.extend_from_slice(&[0xFF, 0xD9]);
        v
    }

    #[test]
    fn reads_sof() {
        let info = jpeg_info(&tiny_jpeg()).unwrap();
        assert_eq!((info.width, info.height, info.components), (3, 2, 1));
    }

    #[test]
    fn builds_valid_structure() {
        let pdf = build_pdf(&[tiny_jpeg(), tiny_jpeg()]).unwrap();
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.starts_with("%PDF-1.4"));
        assert!(s.contains("/Count 2"));
        assert!(s.contains("/DCTDecode"));
        assert!(s.trim_end().ends_with("%%EOF"));
    }
}
