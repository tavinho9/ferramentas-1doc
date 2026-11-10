//! `ThreadPoolExecutor(max_workers=N).map(f, itens)`: N threads processam os
//! itens e os resultados são entregues NA ORDEM dos itens. O primeiro erro
//! (na ordem) interrompe a entrega e as threads param de pegar itens novos.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;

use crate::erro::Res;

pub fn mapear<T, R, F, G>(itens: &[T], workers: usize, f: F, mut em_ordem: G) -> Res<()>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> Res<R> + Sync,
    G: FnMut(usize, R) -> Res<()>,
{
    if itens.is_empty() {
        return Ok(());
    }
    let proximo = AtomicUsize::new(0);
    let parar = AtomicBool::new(false);
    std::thread::scope(|s| {
        let (tx, rx) = mpsc::channel::<(usize, Res<R>)>();
        for _ in 0..workers.max(1).min(itens.len()) {
            let tx = tx.clone();
            let (proximo, parar, f) = (&proximo, &parar, &f);
            s.spawn(move || {
                loop {
                    if parar.load(Ordering::SeqCst) {
                        break;
                    }
                    let i = proximo.fetch_add(1, Ordering::SeqCst);
                    if i >= itens.len() {
                        break;
                    }
                    let r = f(&itens[i]);
                    if tx.send((i, r)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        let mut pendentes: BTreeMap<usize, Res<R>> = BTreeMap::new();
        let mut esperado = 0;
        let resultado = (|| {
            for (i, r) in rx.iter() {
                pendentes.insert(i, r);
                while let Some(r) = pendentes.remove(&esperado) {
                    em_ordem(esperado, r?)?;
                    esperado += 1;
                }
            }
            Ok(())
        })();
        if resultado.is_err() {
            parar.store(true, Ordering::SeqCst);
        }
        resultado
    })
}

/// Contador de ocorrências na ordem do `Counter(...).most_common()`:
/// mais frequentes primeiro; empate, a primeira que apareceu.
pub fn mais_comuns(itens: impl IntoIterator<Item = String>) -> Vec<(String, usize)> {
    let mut v: Vec<(String, usize)> = Vec::new();
    for x in itens {
        match v.iter_mut().find(|(k, _)| *k == x) {
            Some((_, n)) => *n += 1,
            None => v.push((x, 1)),
        }
    }
    v.sort_by_key(|x| std::cmp::Reverse(x.1));
    v
}

/// `ThreadPoolExecutor` + `as_completed`: os resultados são entregues na
/// ordem em que ficam prontos, com o índice do item. O primeiro erro
/// interrompe a entrega e as threads param de pegar itens novos.
pub fn conforme_terminam<T, R, F, G>(
    itens: &[T],
    workers: usize,
    f: F,
    mut ao_receber: G,
) -> Res<()>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> Res<R> + Sync,
    G: FnMut(usize, R) -> Res<()>,
{
    if itens.is_empty() {
        return Ok(());
    }
    let proximo = AtomicUsize::new(0);
    let parar = AtomicBool::new(false);
    std::thread::scope(|s| {
        let (tx, rx) = mpsc::channel::<(usize, Res<R>)>();
        for _ in 0..workers.max(1).min(itens.len()) {
            let tx = tx.clone();
            let (proximo, parar, f) = (&proximo, &parar, &f);
            s.spawn(move || {
                while !parar.load(Ordering::SeqCst) {
                    let i = proximo.fetch_add(1, Ordering::SeqCst);
                    if i >= itens.len() {
                        break;
                    }
                    if tx.send((i, f(&itens[i]))).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        let resultado = (|| {
            for (i, r) in rx.iter() {
                ao_receber(i, r?)?;
            }
            Ok(())
        })();
        if resultado.is_err() {
            parar.store(true, Ordering::SeqCst);
        }
        resultado
    })
}
