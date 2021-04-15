#![feature(trait_alias)]
pub mod lockingmap;
pub mod nonlockingmap;
pub mod nonpointermap;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
