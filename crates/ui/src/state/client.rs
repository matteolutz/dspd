pub struct DspdHttpClient {
    server_url: reqwest::Url,
    client: reqwest::blocking::Client,
}

impl DspdHttpClient {
    pub fn new(server_url: impl Into<reqwest::Url>) -> Self {
        let server_url = server_url.into();
        Self {
            server_url,
            client: reqwest::blocking::Client::new(),
        }
    }

    pub fn request(
        &self,
        method: reqwest::Method,
        path: impl AsRef<str>,
    ) -> reqwest::blocking::RequestBuilder {
        let url = self
            .server_url
            .join(path.as_ref())
            .expect("Failed join URL");
        self.client.request(method, url)
    }

    pub fn get(&self, path: impl AsRef<str>) -> reqwest::blocking::RequestBuilder {
        self.request(reqwest::Method::GET, path)
    }

    pub fn post(&self, path: impl AsRef<str>) -> reqwest::blocking::RequestBuilder {
        self.request(reqwest::Method::POST, path)
    }
}

pub trait RequestBuilderExt {
    fn content_type_json(self) -> Self;
}

impl RequestBuilderExt for reqwest::blocking::RequestBuilder {
    fn content_type_json(self) -> Self {
        self.header("Content-Type", "application/json")
    }
}
