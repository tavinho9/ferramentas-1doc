//! Cancelamento (botão Cancelar da interface): `sessao.CANCELAR` dos scripts.

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::erro::{Erro, Res};

#[derive(Default)]
pub struct Cancelar {
    pedido: Mutex<bool>,
    sinal: Condvar,
}

impl Cancelar {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pedir(&self) {
        *self.pedido.lock().unwrap() = true;
        self.sinal.notify_all();
    }

    pub fn limpar(&self) {
        *self.pedido.lock().unwrap() = false;
    }

    pub fn pedido(&self) -> bool {
        *self.pedido.lock().unwrap()
    }

    /// `checar_cancelamento()`: erro `Cancelado` se a pessoa pediu para parar.
    pub fn checar(&self) -> Res<()> {
        if self.pedido() {
            Err(Erro::Cancelado)
        } else {
            Ok(())
        }
    }

    /// `esperar(segundos)`: dorme, mas acorda (com `Cancelado`) se a pessoa cancelar.
    pub fn esperar(&self, segundos: f64) -> Res<()> {
        let fim = Instant::now() + Duration::from_secs_f64(segundos.max(0.0));
        let mut pedido = self.pedido.lock().unwrap();
        loop {
            if *pedido {
                return Err(Erro::Cancelado);
            }
            let agora = Instant::now();
            if agora >= fim {
                return Ok(());
            }
            pedido = self.sinal.wait_timeout(pedido, fim - agora).unwrap().0;
        }
    }
}
