pub struct Asset {
    pub body: &'static str,
    pub content_type: &'static str,
}

pub fn asset(path: &str) -> Option<Asset> {
    match path {
        "/" | "/index.html" => Some(Asset {
            body: include_str!("ui/index.html"),
            content_type: "text/html; charset=utf-8",
        }),
        "/assets/styles.css" => Some(Asset {
            body: include_str!("ui/styles.css"),
            content_type: "text/css; charset=utf-8",
        }),
        "/assets/app.js" => Some(Asset {
            body: include_str!("ui/app.js"),
            content_type: "application/javascript; charset=utf-8",
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::asset;

    #[test]
    fn serves_embedded_ui_assets() {
        let html = asset("/").expect("index html");
        assert_eq!(html.content_type, "text/html; charset=utf-8");
        assert!(html.body.contains("<title>atrail</title>"));

        let js = asset("/assets/app.js").expect("app js");
        assert_eq!(js.content_type, "application/javascript; charset=utf-8");

        assert!(asset("/missing").is_none());
    }
}
