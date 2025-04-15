mod common;
mod config;
mod proxy;

use crate::config::Config;
use crate::proxy::*;

use std::collections::HashMap;
use base64::{engine::general_purpose::URL_SAFE, Engine as _};
use serde_json::json;
use uuid::Uuid;
use worker::*;
use once_cell::sync::Lazy;
use regex::Regex;


static PROXYIP_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r"^.+-\d+$").unwrap());
static PROXYKV_PATTERN_C1: Lazy<Regex> = Lazy::new(|| Regex::new(r"^([A-Z]{2})\d+$").unwrap());
static PROXYKV_PATTERN_2: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Z]{2}$").unwrap());
static PROXYKV_PATTERN_5: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Z]{5}$").unwrap());


    // Jika cocok dengan regex C1 (format seperti ID1, US20, dst)
    
#[event(fetch)]
async fn main(req: Request, env: Env, _: Context) -> Result<Response> {
    let uuid = env
        .var("UUID")
        .map(|x| Uuid::parse_str(&x.to_string()).unwrap_or_default())?;
    let host = req.url()?.host().map(|x| x.to_string()).unwrap_or_default();
    let main_page_url = env.var("MAIN_PAGE_URL").map(|x| x.to_string()).unwrap();
    let sub_page_url = env.var("SUB_PAGE_URL").map(|x| x.to_string()).unwrap();
    let link_page_url = env.var("LINK_PAGE_URL").map(|x| x.to_string()).unwrap();

    let config = Config { 
        uuid, 
        host: host.clone(), 
        proxy_addr: host, 
        proxy_port: 443, 
        main_page_url, 
        sub_page_url,
        link_page_url
    };

    Router::with_data(config)
        .on_async("/", fe)
        .on_async("/sub-api", sub)
        .on_async("/api-check", link)
        .on_async("/:proxyip", tunnel)
        .run(req, env)
        .await
}

async fn get_response_from_url(url: String) -> Result<Response> {
    let req = Fetch::Url(Url::parse(url.as_str())?);
    let mut res = req.send().await?;
    Response::from_html(res.text().await?)
}

async fn fe(_: Request, cx: RouteContext<Config>) -> Result<Response> {
    get_response_from_url(cx.data.main_page_url.clone()).await
}

async fn sub(_: Request, cx: RouteContext<Config>) -> Result<Response> {
    get_response_from_url(cx.data.sub_page_url.clone()).await
}

async fn link(_: Request, cx: RouteContext<Config>) -> Result<Response> {
    get_response_from_url(cx.data.link_page_url.clone()).await
}

async fn tunnel(req: Request, mut cx: RouteContext<Config>) -> Result<Response> {
    let mut proxyip = cx.param("proxyip").unwrap().to_string();
    if PROXYKV_PATTERN_C1.is_match(&proxyip) {
        let country_code = proxyip.chars().take(2).collect::<String>();  // Ambil dua huruf pertama, misalnya "ID", "US", dsb.
        
        let txt_url = "https://raw.githubusercontent.com/tedjo877/cek/main/update_proxyip.txt";
        let req = Fetch::Url(Url::parse(txt_url)?);
        let mut res = req.send().await?;
        if res.status_code() != 200 {
            return Err(Error::from("Error fetching ip.txt"));
        }
        
        let ip_txt = res.text().await?;
        let lines: Vec<&str> = ip_txt.lines().collect();

        // Filter IP sesuai dengan regex C1 dan kode negara
        let filtered: Vec<(String, u16)> = lines
            .iter()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split(',').collect();
                if parts.len() >= 4 && parts[2] == country_code {
                    if let Ok(port) = parts[1].parse::<u16>() {
                        Some((parts[0].to_string(), port))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        if !filtered.is_empty() {
    let rand_ip = &filtered[rand::random::<usize>() % filtered.len()];
    proxyip = rand_ip.0.clone().replace(":", "-"); // penting!
    let proxy_port = rand_ip.1;
    cx.data.proxy_addr = proxyip.clone(); // ini nanti tetap pakai - dan diproses
    cx.data.proxy_port = proxy_port;


        } else {
            return Err(Error::from("No matching proxy IP found"));
        }
    }

    // Jika cocok dengan regex PROXYIP_PATTERN
    if PROXYIP_PATTERN.is_match(&proxyip) {
        if let Some((addr, port_str)) = proxyip.split_once('-') {
            if let Ok(port) = port_str.parse() {
                cx.data.proxy_addr = addr.to_string();
                cx.data.proxy_port = port;
            }
        }
    }

    if PROXYKV_PATTERN_2.is_match(&proxyip) || PROXYKV_PATTERN_5.is_match(&proxyip) {
        let kvid_list: Vec<String> = proxyip.split(',').map(|s| s.to_string()).collect();
        let kv = cx.kv("V2RAY")?;
        let mut rand_buf = [0u8; 1];
        getrandom::getrandom(&mut rand_buf).expect("failed generating random number");

        // Tentukan URL dan key cache berdasarkan pattern
        let (proxy_url, cache_key) = if PROXYKV_PATTERN_2.is_match(&proxyip) {
            (
                "https://raw.githubusercontent.com/tedjo877/cek/main/update_proxyip.json",
                "proxy_kv_2",
            )
        } else {
            (
                "https://raw.githubusercontent.com/tedjo877/cek/main/update_proxyip8.json",
                "proxy_kv_5",
            )
        };

        let mut proxy_kv_str = kv.get(cache_key).text().await?.unwrap_or_default();

        if proxy_kv_str.is_empty() {
            console_log!("getting proxy kv from github: {}", proxy_url);
            let req = Fetch::Url(Url::parse(proxy_url)?);
            let mut res = req.send().await?;
            if res.status_code() == 200 {
                proxy_kv_str = res.text().await?;
                kv.put(cache_key, &proxy_kv_str)?.expiration_ttl(60 * 60 * 24).execute().await?;
            } else {
                return Err(Error::from(format!("error getting proxy kv: {}", res.status_code())));
            }
        }

        let proxy_kv: HashMap<String, Vec<String>> = serde_json::from_str(&proxy_kv_str)?;

        let kv_index = (rand_buf[0] as usize) % kvid_list.len();
        proxyip = kvid_list[kv_index].clone();

        if let Some(ips) = proxy_kv.get(&proxyip) {
            if !ips.is_empty() {
                let proxyip_index = (rand_buf[0] as usize) % ips.len();
                proxyip = ips[proxyip_index].clone().replace(":", "-");
            }
        }
    }

    if PROXYIP_PATTERN.is_match(&proxyip) {
        if let Some((addr, port_str)) = proxyip.split_once('-') {
            if let Ok(port) = port_str.parse() {
                cx.data.proxy_addr = addr.to_string();
                cx.data.proxy_port = port;
            }
        }
    }

    let upgrade = req.headers().get("Upgrade")?.unwrap_or_default();
    if upgrade.to_lowercase() == "websocket" {
        let WebSocketPair { server, client } = WebSocketPair::new()?;
        server.accept()?;

        wasm_bindgen_futures::spawn_local(async move {
            let events = server.events().unwrap();
            if let Err(e) = ProxyStream::new(cx.data, &server, events).process().await {
                console_log!("[tunnel]: {}", e);
            }
        });

        Response::from_websocket(client)
    } else {
        Response::from_html("hi from wasm!")
    }
}
