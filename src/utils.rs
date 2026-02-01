use actix_web::{HttpResponse, HttpRequest};
use serde::Serialize;
use zip::ZipArchive;

#[derive(Serialize)]
pub struct ApiResponse<T> {
    pub code: i32,
    pub data: T,
}

pub fn json_response<T: Serialize>(data: T) -> HttpResponse {
    HttpResponse::Ok()
        .content_type("application/json; charset=utf-8")
        .append_header(("X-API-Version", "v1"))
        .json(ApiResponse { code: 200, data })
}

pub fn accepted_json_response<T: Serialize>(data: T) -> HttpResponse {
    HttpResponse::Accepted()
        .content_type("application/json; charset=utf-8")
        .append_header(("X-API-Version", "v1"))
        .json(ApiResponse { code: 202, data })
}

pub fn extract_zip_from_archive<R: std::io::Read + std::io::Seek>(archive: &mut ZipArchive<R>, dest: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let outpath = dest.join(file.name());
        
        if !outpath.starts_with(dest) {
            return Err(format!(
                "Zip slip vulnerability detected: extracted file {:?} attempts to traverse outside of destination {:?}",
                outpath, dest
            ).into());
        }

        if file.name().ends_with('/') {
            std::fs::create_dir_all(&outpath)?;
        } else {
            if let Some(p) = outpath.parent() {
                if !p.exists() {
                    std::fs::create_dir_all(p)?;
                }
            }
            let mut outfile = std::fs::File::create(&outpath)?;
            std::io::copy(&mut file, &mut outfile)?;
            
            // Set permissions on Unix-like systems if available in zip
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = file.unix_mode() {
                    let mut perms = outfile.metadata()?.permissions();
                    perms.set_mode(mode);
                    std::fs::set_permissions(&outpath, perms)?;
                } else {
                    // Fallback if no mode in zip, but generally we want to avoid blind 755
                    // Maybe default to 644 for files if mode is missing?
                    // Or keep 755 if it looks like an executable? 
                    // For now, let's rely on zip having modes (build.rs sets them).
                }
            }
        }
    }
    Ok(())
}

pub fn copy_dir_all(src: impl AsRef<std::path::Path>, dst: impl AsRef<std::path::Path>) -> std::io::Result<()> {
    std::fs::create_dir_all(&dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}

pub fn get_client_ip(req: &HttpRequest) -> String {
    if let Some(addr) = req.peer_addr() {
        return addr.ip().to_string();
    }

    if let Some(realip) = req.connection_info().realip_remote_addr() {
        if let Some((ip, _port)) = realip.rsplit_once(':') {
            if ip.contains('.') {
                return ip.to_string();
            }
        }
        return realip.to_string();
    }

    "unknown".to_string()
}
