use url::Url;
fn main() {
    let u = Url::parse("http://0x7f000001/").unwrap();
    println!("{:?}", u.host());
    
    let u2 = Url::parse("http://127.1/").unwrap();
    println!("{:?}", u2.host());
    
    let u3 = Url::parse("http://0177.0.0.01/").unwrap();
    println!("{:?}", u3.host());
}
