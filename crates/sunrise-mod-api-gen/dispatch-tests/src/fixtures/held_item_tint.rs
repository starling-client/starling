struct HeldTintProbe(u32, Arc<Mutex<Vec<u32>>>);

impl ItemPart for HeldTintProbe {
    fn item_subscribed(&self) -> &'static [Option<HookId>] {
        &[Some(HookId::HeldItemTint)]
    }

    fn held_item_tint(&self, _: &dyn ClientHost, event: &mut HeldItemTintEvent<'_>) -> Option<u32> {
        assert_eq!(event.stack.item().0, "fixture:shield");
        assert_eq!(event.stack.count(), 1);
        assert_eq!(event.partial_tick, 0.25);
        self.1.lock().unwrap().push(self.0);
        match (self.0, event.entity_id, event.hand) {
            (1, 11, Hand::Off) => Some(0),
            (2, 11, Hand::Main | Hand::Off) => Some(0xAB_CD_EF),
            (1, _, _) => {
                event.entity_id = 0;
                event.hand = Hand::Off;
                event.partial_tick = 0.0;
                None
            }
            _ => None,
        }
    }
}

#[test]
fn held_item_tint_first_answer_is_scoped_to_holder_and_hand() {
    install(Vec::<ModEntry>::new().into());
    let heard = Arc::new(Mutex::new(Vec::new()));
    install_parts(Parts {
        items: [1, 2]
            .map(|id| leak(HeldTintProbe(id, heard.clone())) as &dyn ItemPart)
            .into(),
        ..Default::default()
    });
    let stack = ItemStack {
        item: "fixture:shield".into(),
        count: 1,
    };
    for (entity, hand, expected, order) in [
        (11, Hand::Off, Some(0), &[1][..]),
        (11, Hand::Main, Some(0xAB_CD_EF), &[1, 2][..]),
        (12, Hand::Off, None, &[1, 2][..]),
    ] {
        let mut tint = None;
        dispatch_held_item_tint(&TestHost, entity, hand, &stack, 0.25, &mut tint);
        assert_eq!(tint, expected);
        assert_eq!(std::mem::take(&mut *heard.lock().unwrap()), order);
    }
    install_parts(Parts::default());
}
