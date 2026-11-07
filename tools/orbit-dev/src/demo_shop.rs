//! Deterministic offline shop; intentionally has no payment or order implementation.
use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};
struct Product {
    sku: &'static str,
    name: &'static str,
    price: u64,
    seller: &'static str,
    rating: &'static str,
    shipping: u64,
}
const PRODUCTS: &[Product] = &[
    Product {
        sku: "charger-65w",
        name: "65W USB-C PD Laptop Charger with 2m Cable",
        price: 3999,
        seller: "Demo Electronics",
        rating: "4.8",
        shipping: 0,
    },
    Product {
        sku: "charger-100w",
        name: "100W USB-C PD Laptop Charger with Cable",
        price: 5999,
        seller: "Demo Power Supply",
        rating: "4.7",
        shipping: 0,
    },
    Product {
        sku: "charger-45w",
        name: "45W USB-C Travel Charger",
        price: 2999,
        seller: "Demo Travel Gear",
        rating: "4.2",
        shipping: 499,
    },
];
fn money(cents: u64) -> String {
    format!("${}.{:02}", cents / 100, cents % 100)
}
fn product(sku: &str) -> Option<&'static Product> {
    PRODUCTS.iter().find(|p| p.sku == sku)
}
fn totals(cart: &[String]) -> (u64, u64, u64, u64) {
    let subtotal = cart
        .iter()
        .filter_map(|s| product(s))
        .map(|p| p.price)
        .sum::<u64>();
    let shipping = cart
        .iter()
        .filter_map(|s| product(s))
        .map(|p| p.shipping)
        .sum::<u64>();
    let tax = (subtotal * 8 + 50) / 100;
    (subtotal, shipping, tax, subtotal + shipping + tax)
}
fn state_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("NEXODE_DEMO_SHOP_STATE") {
        let path = PathBuf::from(path);
        if let Ok(suffix) = path.strip_prefix("~") {
            return Ok(dirs::home_dir()
                .context("home directory unavailable")?
                .join(suffix));
        }
        return Ok(path);
    }
    #[cfg(unix)]
    {
        Ok(
            std::env::temp_dir().join(format!("nexode-demo-shop-{}.json", unsafe {
                libc::getuid()
            })),
        )
    }
    #[cfg(not(unix))]
    {
        Ok(dirs::data_local_dir()
            .context("user data directory unavailable")?
            .join("nexode/demo-shop.json"))
    }
}
fn load(path: &Path) -> Vec<String> {
    fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["cart"].as_array().cloned())
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .filter(|s| product(s).is_some())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
fn save(path: &Path, cart: &[String]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    writeln!(
        file,
        "{}",
        serde_json::to_string_pretty(&json!({"cart":cart}))?
    )?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}
fn render(cart: &[String]) {
    if cart.is_empty() {
        println!("Demo cart is empty.");
        return;
    }
    for sku in cart {
        if let Some(p) = product(sku) {
            println!("1 x {} ({sku}): {}", p.name, money(p.price));
        }
    }
    let (subtotal, shipping, tax, total) = totals(cart);
    println!(
        "Subtotal: {}\nShipping: {}\nSimulated tax: {}\nCheckout total: {}",
        money(subtotal),
        money(shipping),
        money(tax),
        money(total)
    );
}
fn checkout(cart: &[String]) -> i32 {
    if cart.is_empty() {
        eprintln!("Checkout unavailable because the demo cart is empty.");
        return 2;
    }
    println!("DEMO CHECKOUT");
    render(cart);
    println!(
        "\nCHECKOUT PAUSED: no payment method is available.\nThe demo shop can search products, manage the cart, and calculate the final total, but no payment method is configured and it cannot accept payment credentials.\nThe verified cart is preserved. Stop before creating or funding a prepaid card, making a payment, or placing the order.\nNo account was created, no payment data was accepted, and no order was placed."
    );
    3
}
#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    Reset,
    Search {
        #[arg(required=true,num_args=1..)]
        query: Vec<String>,
        #[arg(long)]
        max_price: Option<f64>,
    },
    Show {
        sku: String,
    },
    Add {
        sku: String,
    },
    Cart,
    Checkout,
    PrepareCheckout {
        sku: String,
        #[arg(long)]
        max_total: Option<f64>,
    },
}
fn execute(args: Args, path: &Path) -> Result<i32> {
    match args.command {
        Action::Reset => {
            save(path, &[])?;
            println!("Demo shop reset. Cart is empty. No payment data or orders exist.");
        }
        Action::Search { query, max_price } => {
            let terms = query.join(" ").to_lowercase();
            let mut matches: Vec<_> = PRODUCTS
                .iter()
                .filter(|p| {
                    terms
                        .split_whitespace()
                        .all(|t| p.name.to_lowercase().contains(t))
                        && max_price.is_none_or(|m| p.price as f64 / 100.0 <= m)
                })
                .collect();
            matches.sort_by_key(|p| p.price);
            if matches.is_empty() {
                println!("No matching demo products.");
                return Ok(1);
            }
            println!("DEMO SHOP RESULTS");
            for p in matches {
                println!(
                    "{}: {} | {} | shipping {} | seller {} | rating {}/5",
                    p.sku,
                    p.name,
                    money(p.price),
                    money(p.shipping),
                    p.seller,
                    p.rating
                );
            }
            println!("Add one with: nexode-demo-shop add <sku>");
        }
        Action::Show { sku } => {
            let Some(p) = product(&sku) else {
                eprintln!("Unknown demo SKU: {sku}");
                return Ok(2);
            };
            println!(
                "SKU: {sku}\nProduct: {}\nPrice: {}\nShipping: {}\nSeller: {}\nRating: {}/5\nUSB-C Power Delivery: yes\nCable included: yes\nThis is simulated catalog data. No network request was made.",
                p.name,
                money(p.price),
                money(p.shipping),
                p.seller,
                p.rating
            );
        }
        Action::Add { sku } => {
            if product(&sku).is_none() {
                eprintln!("Unknown demo SKU: {sku}");
                return Ok(2);
            }
            save(path, std::slice::from_ref(&sku))?;
            println!(
                "Added {sku}. The demo cart contains one item.\nReview it with: nexode-demo-shop cart\nContinue with: nexode-demo-shop checkout\nThe simulated checkout only calculates the final total and reports payment availability; it cannot create a card, charge anything, or place an order."
            );
        }
        Action::Cart => {
            println!("DEMO CART");
            render(&load(path));
        }
        Action::Checkout => return Ok(checkout(&load(path))),
        Action::PrepareCheckout { sku, max_total } => {
            if product(&sku).is_none() {
                eprintln!("Unknown demo SKU: {sku}");
                return Ok(2);
            }
            let cart = vec![sku];
            let total = totals(&cart).3;
            if let Some(limit) = max_total {
                ensure!(limit.is_finite() && limit >= 0.0, "invalid maximum total");
                if total as f64 / 100.0 > limit {
                    eprintln!("Checkout total {} exceeds limit ${limit:.2}.", money(total));
                    return Ok(2);
                }
            }
            save(path, &cart)?;
            return Ok(checkout(&cart));
        }
    }
    Ok(0)
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let args = Args::parse_from(std::iter::once("nexode-demo-shop".into()).chain(args));
    let status = execute(args, &state_path()?)?;
    if status != 0 {
        std::process::exit(status);
    }
    Ok(())
}
pub fn session(mut args: impl Iterator<Item = String>) -> Result<()> {
    let before = args.next().context("before file required")?;
    let after = args.next().context("after file required")?;
    let directory = args.next().context("demo directory required")?;
    ensure!(args.next().is_none(), "unexpected arguments");
    let read = |path: String| -> Vec<Value> {
        fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v["clients"].as_array().cloned())
            .unwrap_or_default()
    };
    let before = read(before);
    let ids: HashSet<_> = before
        .iter()
        .filter_map(|v| v["session_id"].as_str())
        .collect();
    let after = read(after);
    let mut candidates: Vec<_> = after
        .iter()
        .filter(|v| {
            v["working_dir"] == directory
                && v["session_id"].as_str().is_some_and(|id| !ids.contains(id))
        })
        .collect();
    candidates.sort_by_key(|v| v["connected_secs_ago"].as_u64().unwrap_or(999999));
    if let Some(v) = candidates.first() {
        println!("{}", v["session_id"].as_str().context("session id")?);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cents_are_deterministic_and_corrupt_state_is_safe() {
        assert_eq!(totals(&["charger-65w".into()]), (3999, 0, 320, 4319));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state");
        fs::write(&path, r#"{"cart":["missing",null,"charger-65w"]}"#).unwrap();
        assert_eq!(load(&path), vec!["charger-65w"]);
        fs::write(&path, "bad").unwrap();
        assert!(load(&path).is_empty());
    }
}
