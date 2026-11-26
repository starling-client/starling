//! Client tooltip components, factories, font measurement, and positioning
//! (vanilla `ClientTooltipComponent`, `ClientTooltipPositioner`, `DefaultTooltipPositioner`,
//! and NeoForge's `RegisterClientTooltipComponentFactoriesEvent`).

use crate::{
    Attached, DrawContext, HookId, Identifier, ItemPart, ItemStack, Text, TextFormError,
    TooltipElement,
    callback_owner::{CallbackOwner, contain_unwind},
    runtime::{self, TooltipProducer},
};
use std::{
    any::{Any, TypeId},
    fmt,
    sync::{Arc, OnceLock},
};

/// Marker trait for typed tooltip component data (vanilla `TooltipComponent`).
///
/// Wrap a value in [`TooltipComponent::custom`] and return it from `ItemPart::tooltip_image` or
/// push [`crate::TooltipElement::Component`] in `render_tooltip_gather_components`. Register a
/// matching client factory in `register_client_tooltip_component_factories`.
pub trait TooltipComponentData: Any + Send + Sync + 'static {}

struct OwnedTooltipData {
    data: Option<Box<dyn Any + Send + Sync>>,
    creator: Option<&'static str>,
    producer: OnceLock<TooltipProducer>,
}

impl Drop for OwnedTooltipData {
    fn drop(&mut self) {
        if let Some(data) = self.data.take() {
            if let Some(producer) = self.producer.get() {
                producer.dispose(data);
            } else if let Some(creator) = self.creator {
                CallbackOwner::for_mod(creator).dispose("tooltip_component", "data_drop", data);
            } else {
                let _ = contain_unwind(move || drop(data));
            }
        }
    }
}

#[derive(Clone)]
enum Repr {
    Bundle(Arc<ItemStack>),
    Custom {
        type_id: TypeId,
        type_name: &'static str,
        data: Arc<OwnedTooltipData>,
    },
}

// no wire: carries in-process typed tooltip component data.
// exhaustive: opaque wrapper with private representation.
/// A tooltip component's data payload before client preparation (vanilla `TooltipComponent`).
#[derive(Clone)]
pub struct TooltipComponent {
    repr: Repr,
}

impl TooltipComponent {
    /// Wrap a bundle stack for vanilla's built-in bundle contents tooltip component.
    pub fn bundle(stack: ItemStack) -> Self {
        Self {
            repr: Repr::Bundle(Arc::new(stack)),
        }
    }

    /// Wrap mod-defined [`TooltipComponentData`] for resolution by a factory registered in
    /// `register_client_tooltip_component_factories`.
    pub fn custom<T: TooltipComponentData>(data: T) -> Self {
        Self {
            repr: Repr::Custom {
                type_id: TypeId::of::<T>(),
                type_name: std::any::type_name::<T>(),
                data: Arc::new(OwnedTooltipData {
                    data: Some(Box::new(data)),
                    creator: crate::content::crash::current_native_mod(),
                    producer: OnceLock::new(),
                }),
            },
        }
    }

    /// Borrow the bundle stack if this is a built-in bundle component.
    pub fn as_bundle(&self) -> Option<&ItemStack> {
        match &self.repr {
            Repr::Bundle(stack) => Some(stack.as_ref()),
            Repr::Custom { .. } => None,
        }
    }

    /// Downcast a custom tooltip component payload to its concrete [`TooltipComponentData`] type.
    pub fn downcast_ref<T: TooltipComponentData>(&self) -> Option<&T> {
        match &self.repr {
            Repr::Custom { data, .. } => data.data.as_deref().and_then(|d| d.downcast_ref::<T>()),
            Repr::Bundle(_) => None,
        }
    }

    /// The [`TypeId`] of the custom [`TooltipComponentData`], or `None` for a built-in bundle.
    pub fn type_id(&self) -> Option<TypeId> {
        match &self.repr {
            Repr::Custom { type_id, .. } => Some(*type_id),
            Repr::Bundle(_) => None,
        }
    }

    /// The Rust type name of the custom [`TooltipComponentData`], or `None` for a built-in bundle.
    pub fn type_name(&self) -> Option<&'static str> {
        match &self.repr {
            Repr::Custom { type_name, .. } => Some(*type_name),
            Repr::Bundle(_) => None,
        }
    }

    /// Borrow the erased custom data payload for factory dispatch.
    // hidden: engine factory dispatch reads the erased custom payload.
    #[doc(hidden)]
    pub fn __as_any(&self) -> Option<&(dyn Any + Send + Sync)> {
        match &self.repr {
            Repr::Custom { data, .. } => data.data.as_deref(),
            Repr::Bundle(_) => None,
        }
    }

    /// The publisher, or the native mod that constructed an unpublished custom payload.
    // hidden: engine diagnostics attribute preparation errors to the payload producer.
    #[doc(hidden)]
    pub fn __owner_id(&self) -> Option<&'static str> {
        match &self.repr {
            Repr::Custom { data, .. } => data.producer.get().map(|p| p.mod_id).or(data.creator),
            Repr::Bundle(_) => None,
        }
    }
    fn bind_producer(&self, producer: TooltipProducer) {
        if let Repr::Custom { data, .. } = &self.repr {
            // Moving or cloning an existing payload in Gather does not change its publisher.
            data.producer.get_or_init(|| producer);
        }
    }
}

pub(crate) fn retain_gather_tooltip_payloads(mod_id: &'static str, elements: &[TooltipElement]) {
    let producer = TooltipProducer::new(HookId::RenderTooltipGatherComponents, mod_id);
    for element in elements {
        if let TooltipElement::Component(component) = element {
            component.bind_producer(producer);
        }
    }
}

pub(crate) fn invoke_tooltip_image(
    attached: &Attached<dyn ItemPart>,
    call: impl FnOnce(&dyn ItemPart) -> Option<TooltipComponent>,
) -> Option<TooltipComponent> {
    let callback = runtime::Callback::TooltipImage(attached.order);
    if !runtime::mod_index_on(attached.mod_index) || runtime::poisoned(callback, attached.mod_id) {
        return None;
    }
    let _guard = crate::content::crash::enter_native_mod(crate::content::crash::ModCall {
        id: attached.mod_id,
        hook: "tooltip_image",
        part: Some(attached.type_name),
    });
    let component = runtime::invoke(callback, "tooltip_image", attached.mod_id, || {
        call(&*attached.part)
    })??;
    component.bind_producer(TooltipProducer::new(callback, attached.mod_id));
    Some(component)
}

impl PartialEq for TooltipComponent {
    fn eq(&self, other: &Self) -> bool {
        match (&self.repr, &other.repr) {
            (Repr::Bundle(a), Repr::Bundle(b)) => a == b,
            (
                Repr::Custom {
                    type_id: a_id,
                    data: a_data,
                    ..
                },
                Repr::Custom {
                    type_id: b_id,
                    data: b_data,
                    ..
                },
            ) => a_id == b_id && Arc::ptr_eq(a_data, b_data),
            _ => false,
        }
    }
}

impl fmt::Debug for TooltipComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.repr {
            Repr::Bundle(stack) => f
                .debug_tuple("TooltipComponent::Bundle")
                .field(stack)
                .finish(),
            Repr::Custom { type_name, .. } => f
                .debug_struct("TooltipComponent::Custom")
                .field("type_name", type_name)
                .finish_non_exhaustive(),
        }
    }
}

/// Font metrics and line splitting lent to [`ClientTooltipComponent`] and `render_tooltip_pre`
/// (vanilla `Font` subset used during tooltip measurement and drawing).
pub trait TooltipFont {
    /// The active font identifier (for example `minecraft:default`).
    fn id(&self) -> &Identifier;

    /// The height of one line of text in GUI pixels (`Font.lineHeight`, 9 in vanilla).
    fn line_height(&self) -> i32;

    /// Measure `text`'s rendered width in GUI pixels (`Font.width`).
    fn width(&self, text: &Text) -> Result<i32, TextFormError>;

    /// Wrap `text` to `max_width` GUI pixels (`Font.split`).
    fn split(&self, text: &Text, max_width: i32) -> Result<Vec<Text>, TextFormError>;
}

/// A client-side tooltip component that measures and draws itself (vanilla `ClientTooltipComponent`).
///
/// The text pass runs [`draw_text`](Self::draw_text) for every component in order, then the image
/// pass runs [`draw_image`](Self::draw_image) for every component in order.
pub trait ClientTooltipComponent: 'static {
    /// Height of this component in GUI pixels (`ClientTooltipComponent.getHeight`).
    /// The box uses the height measured before drawing. The text and image passes each
    /// measure height again after drawing the component to place the next one.
    fn height(&self, font: &dyn TooltipFont) -> i32;

    /// Width of this component in GUI pixels (`ClientTooltipComponent.getWidth`).
    fn width(&self, font: &dyn TooltipFont) -> i32;

    /// Whether this tooltip should still appear for a hovered container slot while the cursor
    /// holds an item stack (`ClientTooltipComponent.showTooltipWithItemInHand`, default `false`).
    /// Recipe and trade previews do not use this check.
    fn show_tooltip_with_item_in_hand(&self) -> bool {
        false
    }

    /// Draw this component's text at `(x, y)` in GUI pixels during the tooltip's text pass
    /// (`ClientTooltipComponent.extractText`).
    fn draw_text(&self, font: &dyn TooltipFont, x: i32, y: i32, draw: &mut dyn DrawContext) {
        let _ = (font, x, y, draw);
    }

    /// Draw this component's graphics at `(x, y)` during `ClientTooltipComponent.extractImage`.
    /// `width` and `height` are the entire tooltip's content-box dimensions in GUI pixels,
    /// not this component's measurements.
    fn draw_image(
        &self,
        font: &dyn TooltipFont,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        draw: &mut dyn DrawContext,
    ) {
        let _ = (font, x, y, width, height, draw);
    }
}

trait AnyClientTooltipComponent: 'static {
    fn as_any(&self) -> &dyn Any;
    fn height(&self, font: &dyn TooltipFont) -> i32;
    fn width(&self, font: &dyn TooltipFont) -> i32;
    fn show_tooltip_with_item_in_hand(&self) -> bool;
    fn draw_text(&self, font: &dyn TooltipFont, x: i32, y: i32, draw: &mut dyn DrawContext);
    fn draw_image(
        &self,
        font: &dyn TooltipFont,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        draw: &mut dyn DrawContext,
    );
}

impl<C: ClientTooltipComponent + 'static> AnyClientTooltipComponent for C {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn height(&self, font: &dyn TooltipFont) -> i32 {
        ClientTooltipComponent::height(self, font)
    }

    fn width(&self, font: &dyn TooltipFont) -> i32 {
        ClientTooltipComponent::width(self, font)
    }

    fn show_tooltip_with_item_in_hand(&self) -> bool {
        ClientTooltipComponent::show_tooltip_with_item_in_hand(self)
    }

    fn draw_text(&self, font: &dyn TooltipFont, x: i32, y: i32, draw: &mut dyn DrawContext) {
        ClientTooltipComponent::draw_text(self, font, x, y, draw);
    }

    fn draw_image(
        &self,
        font: &dyn TooltipFont,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        draw: &mut dyn DrawContext,
    ) {
        ClientTooltipComponent::draw_image(self, font, x, y, width, height, draw);
    }
}

type ErasedTooltipComponentFactory =
    dyn Fn(&(dyn Any + Send + Sync)) -> Option<Box<dyn AnyClientTooltipComponent>> + Send + Sync;

struct OwnedTooltipFactory {
    create: Option<Box<ErasedTooltipComponentFactory>>,
    owner: Option<CallbackOwner>,
}

impl Drop for OwnedTooltipFactory {
    fn drop(&mut self) {
        if let Some(create) = self.create.take() {
            match self.owner.as_ref() {
                Some(owner) => owner.dispose("tooltip_component", "factory_drop", create),
                None => {
                    let _ = contain_unwind(move || drop(create));
                }
            }
        }
    }
}

// no wire: process-local tooltip component factory closure.
// exhaustive: opaque factory wrapper with private representation.
/// Type-erased client factory mapping one [`TooltipComponentData`] type into a
/// [`PreparedTooltipComponent`].
#[derive(Clone)]
pub struct TooltipComponentFactory {
    type_id: TypeId,
    type_name: &'static str,
    callback: Arc<OwnedTooltipFactory>,
}

impl TooltipComponentFactory {
    /// Wrap a typed factory converting `&T` into a [`ClientTooltipComponent`] `C`.
    /// The mod creating the factory owns its callbacks and the disposal of its captures.
    pub fn new<T: TooltipComponentData, C: ClientTooltipComponent + 'static>(
        factory: impl Fn(&T) -> C + Send + Sync + 'static,
    ) -> Self {
        let owner = crate::content::crash::current_native_mod().map(CallbackOwner::for_mod);
        Self {
            type_id: TypeId::of::<T>(),
            type_name: std::any::type_name::<T>(),
            callback: Arc::new(OwnedTooltipFactory {
                create: Some(Box::new(move |data| {
                    let typed = data.downcast_ref::<T>()?;
                    Some(Box::new(factory(typed)))
                })),
                owner,
            }),
        }
    }

    /// The [`TypeId`] of the [`TooltipComponentData`] this factory handles.
    // hidden: engine factory registry keys factories by TypeId.
    #[doc(hidden)]
    pub fn __type_id(&self) -> TypeId {
        self.type_id
    }

    /// The Rust type name of the [`TooltipComponentData`] this factory handles.
    // hidden: engine diagnostics name the registered component type.
    #[doc(hidden)]
    pub fn __type_name(&self) -> &'static str {
        self.type_name
    }

    /// The mod that owns this factory, for preparation diagnostics.
    // hidden: engine diagnostics attribute failures to the factory's owning mod.
    #[doc(hidden)]
    pub fn __owner_id(&self) -> Option<&'static str> {
        self.callback.owner.as_ref().map(CallbackOwner::id)
    }

    /// Invoke the factory under panic containment, returning `None` if the type mismatches,
    /// the owner is disabled, or the factory panics.
    // hidden: engine tooltip preparation invokes registered factories.
    #[doc(hidden)]
    pub fn __create(&self, data: &(dyn Any + Send + Sync)) -> Option<PreparedTooltipComponent> {
        let create = self.callback.create.as_deref()?;
        let built = match self.callback.owner.as_ref() {
            Some(owner) => owner
                .call("tooltip_component", "create", || create(data))
                .ok(),
            None => contain_unwind(|| create(data)).ok(),
        }??;
        Some(PreparedTooltipComponent {
            repr: PreparedRepr::Custom {
                component: Some(built),
                owner: self.callback.owner.clone(),
            },
        })
    }
}

impl fmt::Debug for TooltipComponentFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TooltipComponentFactory")
            .field("type_name", &self.type_name)
            .finish_non_exhaustive()
    }
}

/// Registrar handed to `register_client_tooltip_component_factories` once at client startup
/// (NeoForge's `RegisterClientTooltipComponentFactoriesEvent`).
pub trait ClientTooltipComponentFactoriesRegistrar {
    /// Register a type-erased tooltip component factory.
    // hidden: object-safe backing method for `impl dyn ClientTooltipComponentFactoriesRegistrar`.
    #[doc(hidden)]
    fn register_factory(&mut self, factory: TooltipComponentFactory);
}

impl dyn ClientTooltipComponentFactoriesRegistrar + '_ {
    /// Register a client factory for tooltip component data type `T`.
    ///
    /// A later registration for the same `T` replaces the earlier one, matching NeoForge's
    /// `RegisterClientTooltipComponentFactoriesEvent.register`.
    pub fn register<T: TooltipComponentData, C: ClientTooltipComponent + 'static>(
        &mut self,
        factory: impl Fn(&T) -> C + Send + Sync + 'static,
    ) {
        self.register_factory(TooltipComponentFactory::new::<T, C>(factory));
    }
}

/// Places a measured tooltip box within the screen bounds (vanilla `ClientTooltipPositioner`).
pub trait ClientTooltipPositioner {
    /// Compute the tooltip's top-left corner `[x, y]` in GUI pixels from the screen size
    /// `[width, height]`, anchor point `cursor`, and measured content box `tooltip_size`.
    fn position_tooltip(
        &self,
        screen: [i32; 2],
        cursor: [i32; 2],
        tooltip_size: [i32; 2],
    ) -> [i32; 2];
}

// no wire: stateless vanilla tooltip positioner.
// exhaustive: stateless unit type matching vanilla DefaultTooltipPositioner.
/// Vanilla's default tooltip positioner (`DefaultTooltipPositioner.INSTANCE`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DefaultTooltipPositioner;

impl ClientTooltipPositioner for DefaultTooltipPositioner {
    fn position_tooltip(
        &self,
        screen: [i32; 2],
        cursor: [i32; 2],
        tooltip_size: [i32; 2],
    ) -> [i32; 2] {
        let mut x = cursor[0].wrapping_add(12);
        let mut y = cursor[1].wrapping_sub(12);
        let w = tooltip_size[0];
        let h = tooltip_size[1];
        if x.wrapping_add(w) > screen[0] {
            x = x.wrapping_sub(24).wrapping_sub(w).max(4);
        }
        let padded_height = h.wrapping_add(3);
        if y.wrapping_add(padded_height) > screen[1] {
            y = screen[1].wrapping_sub(padded_height);
        }
        [x, y]
    }
}

enum PreparedRepr {
    Text {
        text: Text,
        color: u32,
    },
    Bundle {
        stack: ItemStack,
        non_empty_height: Option<i32>,
    },
    Custom {
        component: Option<Box<dyn AnyClientTooltipComponent>>,
        owner: Option<CallbackOwner>,
    },
}

// no wire: in-process prepared tooltip component.
// exhaustive: opaque prepared component with private representation.
/// A tooltip component prepared after `render_tooltip_gather_components` and lent to
/// `render_tooltip_pre` before measurement and drawing.
pub struct PreparedTooltipComponent {
    repr: PreparedRepr,
}

impl Drop for PreparedTooltipComponent {
    fn drop(&mut self) {
        if let PreparedRepr::Custom { component, owner } = &mut self.repr
            && let Some(component) = component.take()
        {
            match owner.as_ref() {
                Some(owner) => owner.dispose("tooltip_component", "drop", component),
                None => {
                    let _ = contain_unwind(move || drop(component));
                }
            }
        }
    }
}

impl PreparedTooltipComponent {
    fn call_custom<R>(
        &self,
        operation: &'static str,
        f: impl FnOnce(&dyn AnyClientTooltipComponent) -> R,
    ) -> Option<R> {
        let PreparedRepr::Custom {
            component: Some(component),
            owner,
        } = &self.repr
        else {
            return None;
        };
        match owner.as_ref() {
            Some(owner) => owner
                .call("tooltip_component", operation, || f(&**component))
                .ok(),
            None => contain_unwind(|| f(&**component)).ok(),
        }
    }

    /// Measured width of this component in GUI pixels using `font`.
    /// `None` means measurement failed, including when a custom component is disabled or panics.
    pub fn width(&self, font: &dyn TooltipFont) -> Option<i32> {
        match &self.repr {
            PreparedRepr::Text { text, .. } => font.width(text).ok(),
            PreparedRepr::Bundle { .. } => Some(96),
            PreparedRepr::Custom { .. } => self.call_custom("width", |c| c.width(font)),
        }
    }

    /// Measured height of this component in GUI pixels using `font`.
    /// `None` means measurement failed, including when a custom component is disabled or panics.
    pub fn height(&self, font: &dyn TooltipFont) -> Option<i32> {
        match &self.repr {
            PreparedRepr::Text { .. } => Some(10),
            PreparedRepr::Bundle {
                non_empty_height, ..
            } => match *non_empty_height {
                Some(height) => Some(height),
                None => {
                    let desc =
                        Text::translate("item.minecraft.bundle.empty.description", Vec::new());
                    let lines = font.split(&desc, 96).ok()?;
                    Some(
                        (lines.len() as i32)
                            .wrapping_mul(font.line_height())
                            .wrapping_add(21),
                    )
                }
            },
            PreparedRepr::Custom { .. } => self.call_custom("height", |c| c.height(font)),
        }
    }

    /// Whether this component keeps the tooltip visible while the cursor holds an item stack.
    /// Returns `false` if a custom component is disabled or panics.
    pub fn show_tooltip_with_item_in_hand(&self) -> bool {
        match &self.repr {
            PreparedRepr::Text { .. } => false,
            PreparedRepr::Bundle { .. } => true,
            PreparedRepr::Custom { .. } => self
                .call_custom("show_tooltip_with_item_in_hand", |c| {
                    c.show_tooltip_with_item_in_hand()
                })
                .unwrap_or(false),
        }
    }

    /// Borrow the text and colour if this is a built-in text line component.
    pub fn as_text(&self) -> Option<(&Text, u32)> {
        match &self.repr {
            PreparedRepr::Text { text, color } => Some((text, *color)),
            _ => None,
        }
    }

    /// Borrow the bundle stack if this is a built-in bundle tooltip component.
    pub fn as_bundle(&self) -> Option<&ItemStack> {
        match &self.repr {
            PreparedRepr::Bundle { stack, .. } => Some(stack),
            _ => None,
        }
    }

    /// Downcast a custom prepared component to its concrete [`ClientTooltipComponent`] type.
    pub fn downcast_ref<C: ClientTooltipComponent + 'static>(&self) -> Option<&C> {
        match &self.repr {
            PreparedRepr::Custom {
                component: Some(component),
                ..
            } => component.as_any().downcast_ref::<C>(),
            _ => None,
        }
    }

    /// Build a prepared built-in text line component.
    // hidden: engine tooltip preparation constructs built-in text components.
    #[doc(hidden)]
    pub fn __text(text: Text, color: u32) -> Self {
        Self {
            repr: PreparedRepr::Text { text, color },
        }
    }

    /// Build a prepared built-in bundle tooltip component (`non_empty_height` is `None` when empty).
    // hidden: engine tooltip preparation constructs built-in bundle components.
    #[doc(hidden)]
    pub fn __bundle(stack: ItemStack, non_empty_height: Option<i32>) -> Self {
        Self {
            repr: PreparedRepr::Bundle {
                stack,
                non_empty_height,
            },
        }
    }

    /// Run a custom component's text pass under panic containment.
    /// Returns `false` if a custom component is disabled or panicked so the caller discards staged paint.
    // hidden: engine tooltip rendering runs custom component text passes.
    #[doc(hidden)]
    pub fn __draw_text(
        &self,
        font: &dyn TooltipFont,
        x: i32,
        y: i32,
        draw: &mut dyn DrawContext,
    ) -> bool {
        match &self.repr {
            PreparedRepr::Custom { .. } => self
                .call_custom("draw_text", |c| c.draw_text(font, x, y, draw))
                .is_some(),
            PreparedRepr::Text { .. } | PreparedRepr::Bundle { .. } => true,
        }
    }

    /// Run a custom component's image pass under panic containment.
    /// Returns `false` if a custom component is disabled or panicked so the caller discards staged paint.
    // hidden: engine tooltip rendering runs custom component image passes.
    #[doc(hidden)]
    pub fn __draw_image(
        &self,
        font: &dyn TooltipFont,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        draw: &mut dyn DrawContext,
    ) -> bool {
        match &self.repr {
            PreparedRepr::Custom { .. } => self
                .call_custom("draw_image", |c| {
                    c.draw_image(font, x, y, width, height, draw)
                })
                .is_some(),
            PreparedRepr::Text { .. } | PreparedRepr::Bundle { .. } => true,
        }
    }
}

impl fmt::Debug for PreparedTooltipComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.repr {
            PreparedRepr::Text { text, color } => f
                .debug_struct("PreparedTooltipComponent::Text")
                .field("text", text)
                .field("color", color)
                .finish(),
            PreparedRepr::Bundle {
                stack,
                non_empty_height,
            } => f
                .debug_struct("PreparedTooltipComponent::Bundle")
                .field("stack", stack)
                .field("non_empty_height", non_empty_height)
                .finish(),
            PreparedRepr::Custom { owner, .. } => f
                .debug_struct("PreparedTooltipComponent::Custom")
                .field("owner", &owner.as_ref().map(CallbackOwner::id))
                .finish_non_exhaustive(),
        }
    }
}

impl dyn DrawContext + '_ {
    /// Borrow the tooltip font `font` for measurement or splitting. An unknown font ID
    /// uses missing glyphs, not `minecraft:default`; explicit style fonts still apply.
    pub fn with_tooltip_font<R>(
        &self,
        font: &Identifier,
        f: impl FnOnce(&dyn TooltipFont) -> R,
    ) -> R {
        let mut f = Some(f);
        let mut out = None;
        self.with_tooltip_font_raw(font, &mut |resolved| {
            if let Some(call) = f.take() {
                out = Some(call(resolved));
            }
        });
        out.expect("DrawContext::with_tooltip_font_raw did not invoke its callback")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestFont(Identifier);

    impl TooltipFont for TestFont {
        fn id(&self) -> &Identifier {
            &self.0
        }

        fn line_height(&self) -> i32 {
            9
        }

        fn width(&self, text: &Text) -> Result<i32, TextFormError> {
            match &text.content {
                crate::text::TextContent::Literal(s) => Ok(s.len() as i32 * 6),
                _ => Ok(12),
            }
        }

        fn split(&self, _text: &Text, _max_width: i32) -> Result<Vec<Text>, TextFormError> {
            Ok(vec![Text::literal("a"), Text::literal("b")])
        }
    }

    struct CustomData(i32);
    impl TooltipComponentData for CustomData {}

    struct CustomClient(i32);
    impl ClientTooltipComponent for CustomClient {
        fn height(&self, _font: &dyn TooltipFont) -> i32 {
            self.0
        }

        fn width(&self, _font: &dyn TooltipFont) -> i32 {
            self.0 * 2
        }

        fn show_tooltip_with_item_in_hand(&self) -> bool {
            true
        }
    }

    #[test]
    fn default_positioner_matches_vanilla_bounds_without_overflow() {
        let p = DefaultTooltipPositioner;
        assert_eq!(
            p.position_tooltip([320, 240], [50, 100], [60, 20]),
            [62, 88]
        );
        assert_eq!(
            p.position_tooltip([100, 240], [80, 100], [50, 20]),
            [18, 88]
        );
        assert_eq!(p.position_tooltip([40, 240], [35, 100], [50, 20]), [4, 88]);
        assert_eq!(p.position_tooltip([320, 100], [50, 95], [60, 30]), [62, 67]);
        let _ = p.position_tooltip(
            [i32::MAX, i32::MIN],
            [i32::MAX, i32::MIN],
            [i32::MAX, i32::MAX],
        );
    }

    #[test]
    fn custom_component_factory_prepares_and_contains_panics() {
        let font = TestFont(Identifier::parse("minecraft:default").unwrap());
        let data = TooltipComponent::custom(CustomData(15));
        assert_eq!(data.downcast_ref::<CustomData>().map(|d| d.0), Some(15));
        assert_eq!(data.type_id(), Some(TypeId::of::<CustomData>()));
        assert!(data.type_name().is_some());
        assert_eq!(data, data.clone());
        assert_ne!(data, TooltipComponent::custom(CustomData(15)));

        let factory =
            TooltipComponentFactory::new::<CustomData, CustomClient>(|d| CustomClient(d.0));
        let prepared = factory
            .__create(data.__as_any().unwrap())
            .expect("factory should build component");
        assert_eq!(prepared.width(&font), Some(30));
        assert_eq!(prepared.height(&font), Some(15));
        assert!(prepared.show_tooltip_with_item_in_hand());
        assert_eq!(
            prepared.downcast_ref::<CustomClient>().map(|c| c.0),
            Some(15)
        );

        struct PanickingClient;
        impl ClientTooltipComponent for PanickingClient {
            fn height(&self, _font: &dyn TooltipFont) -> i32 {
                panic!("boom height")
            }

            fn width(&self, _font: &dyn TooltipFont) -> i32 {
                10
            }
        }
        let panicking_owner = CallbackOwner::for_mod("test_tooltip_panic");
        let mut panicking_factory =
            TooltipComponentFactory::new::<CustomData, PanickingClient>(|_| PanickingClient);
        Arc::get_mut(&mut panicking_factory.callback).unwrap().owner =
            Some(panicking_owner.clone());
        let prepared = panicking_factory
            .__create(data.__as_any().unwrap())
            .expect("factory built panicking client");
        assert_eq!(prepared.height(&font), None);
        assert!(!panicking_owner.is_enabled());
        assert_eq!(prepared.width(&font), None);
    }

    #[test]
    fn panicking_payload_drop_is_contained_on_last_release() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct PanicDrop(Arc<AtomicUsize>);
        impl TooltipComponentData for PanicDrop {}
        impl Drop for PanicDrop {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::Relaxed);
                panic!("payload drop boom");
            }
        }

        struct QuietMod;
        impl crate::Hooks for QuietMod {
            fn subscribed(&self) -> &'static [Option<crate::HookId>] {
                &[]
            }
        }

        let drops = Arc::new(AtomicUsize::new(0));
        crate::testing::with_dispatch(
            vec![crate::ModEntry::new(
                "test_payload_drop_owner",
                Box::new(QuietMod),
            )]
            .into(),
            || {
                let _ = crate::__private::take_hook_panics();
                let component = TooltipComponent::custom(PanicDrop(drops.clone()));
                let clone = component.clone();
                retain_gather_tooltip_payloads(
                    "test_payload_drop_owner",
                    &[TooltipElement::Component(clone)],
                );
                assert_eq!(drops.load(Ordering::Relaxed), 0);
                assert!(!runtime::poisoned(
                    HookId::RenderTooltipGatherComponents,
                    "test_payload_drop_owner"
                ));

                // Last Arc release disables the same producer that bound the payload.
                drop(component);
                assert_eq!(drops.load(Ordering::Relaxed), 1);
                assert!(runtime::poisoned(
                    HookId::RenderTooltipGatherComponents,
                    "test_payload_drop_owner"
                ));
                let notices = crate::__private::take_hook_panics();
                assert!(
                    notices
                        .iter()
                        .any(|n| n.mod_id == "test_payload_drop_owner" && n.hook == "data_drop"),
                    "panicking payload drop must be attributed to its producer; got {notices:?}"
                );
            },
        );
    }
}
