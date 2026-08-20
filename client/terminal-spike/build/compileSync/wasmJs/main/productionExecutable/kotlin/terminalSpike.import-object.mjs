
import * as Li9za2lrby5tanM from './skiko.mjs';
import * as d2FzbTpqcy1zdHJpbmc from './terminalSpike.js-builtins.mjs';

const wasmJsTag = WebAssembly.JSTag;
const wasmTag = wasmJsTag ?? new WebAssembly.Tag({ parameters: ['externref'] });

// Placed here to give access to it from externals (js_code)
let wasmExports;
let require;

if (typeof process !== 'undefined' && process.release.name === 'node') {
    const module = await import(/* webpackIgnore: true */'node:module');
    const importMeta = import.meta;
    require = module.default.createRequire(importMeta.url);
}

export function setWasmExports(exports) {
    wasmExports = exports;
}

const _ref_Li9za2lrby5tanM_ = Li9za2lrby5tanM;
const _ref_Li9za2lrby5tanM_c2tpa29BcGk = Li9za2lrby5tanM.skikoApi;

const cachedJsObjects = new WeakMap();
function getCachedJsObject(ref, ifNotCached) {
    if (typeof ref !== 'object' && typeof ref !== 'function') return ifNotCached;
    const cached = cachedJsObjects.get(ref);
    if (cached !== void 0) return cached;
    cachedJsObjects.set(ref, ifNotCached);
    return ifNotCached;
}

const js_code = {
    'kotlin.createJsError' : (message, cause) => new Error(message, { cause }),
    'kotlin.wasm.internal.jsThrow' : wasmTag === wasmJsTag ? (e) => { throw e; } : () => {},
    'kotlin.wasm.internal.getJsEmptyString' : () => '',
    'kotlin.wasm.internal.externrefToInt' : (ref) => Number(ref),
    'kotlin.wasm.internal.externrefToString' : (ref) => String(ref),
    'kotlin.wasm.internal.externrefEquals' : (lhs, rhs) => lhs === rhs,
    'kotlin.wasm.internal.externrefHashCode' : 
    (() => {
    const dataView = new DataView(new ArrayBuffer(8));
    function numberHashCode(obj) {
        if ((obj | 0) === obj) {
            return obj | 0;
        } else {
            dataView.setFloat64(0, obj, true);
            return (dataView.getInt32(0, true) * 31 | 0) + dataView.getInt32(4, true) | 0;
        }
    }

    const hashCodes = new WeakMap();
    function getObjectHashCode(obj) {
        const res = hashCodes.get(obj);
        if (res === undefined) {
            const POW_2_32 = 4294967296;
            const hash = (Math.random() * POW_2_32) | 0;
            hashCodes.set(obj, hash);
            return hash;
        }
        return res;
    }

    function getStringHashCode(str) {
        var hash = 0;
        for (var i = 0; i < str.length; i++) {
            var code  = str.charCodeAt(i);
            hash  = (hash * 31 + code) | 0;
        }
        return hash;
    }

    return (obj) => {
        if (obj == null) {
            return 0;
        }
        switch (typeof obj) {
            case "object":
            case "function":
                return getObjectHashCode(obj);
            case "number":
                return numberHashCode(obj);
            case "boolean":
                return obj ? 1231 : 1237;
            default:
                return getStringHashCode(String(obj)); 
        }
    }
    })(),
    'kotlin.wasm.internal.isNullish' : (ref) => ref == null,
    'kotlin.wasm.internal.getJsTrue' : () => true,
    'kotlin.wasm.internal.getJsFalse' : () => false,
    'kotlin.wasm.internal.kotlinUIntToJsNumberUnsafe' : (x) => x >>> 0,
    'kotlin.wasm.internal.kotlinULongToJsBigIntUnsafe' : (x) => x & 0xFFFFFFFFFFFFFFFFn,
    'kotlin.wasm.internal.getCachedJsObject_$external_fun' : (p0, p1) => getCachedJsObject(p0, p1),
    'kotlin.wasm.internal.itoa32_$external_fun' : (p0) => String(p0),
    'kotlin.wasm.internal.itoa64_$external_fun' : (p0) => String(p0),
    'kotlin.wasm.internal.utoa64_$external_fun' : (p0) => String(p0),
    'kotlin.wasm.internal.utoa32_$external_fun' : (p0) => String(p0),
    'kotlin.js.jsCatch' : (f) => { 
        let result = null;
        try { 
            f();
        } catch (e) {
           result = e;
        }
        return result;
         },
    'kotlin.js.__convertKotlinClosureToJsClosure_(()->Unit)' : (f) => getCachedJsObject(f, () => wasmExports['__callFunction_(()->Unit)'](f, )),
    'kotlin.js.jsThrow' : (e) => { throw e; },
    'kotlin.io.printlnImpl' : (message) => console.log(message),
    'kotlin.js.jsArrayGet' : (array, index) => array[index],
    'kotlin.js.jsArraySet' : (array, index, value) => { array[index] = value },
    'kotlin.js.JsArray_$external_fun' : () => new Array(),
    'kotlin.js.length_$external_prop_getter' : (_this) => _this.length,
    'kotlin.js.stackPlaceHolder_js_code' : () => (''),
    'kotlin.js.message_$external_prop_getter' : (_this) => _this.message,
    'kotlin.js.name_$external_prop_setter' : (_this, v) => _this.name = v,
    'kotlin.js.stack_$external_prop_getter' : (_this) => _this.stack,
    'kotlin.js.kotlinException_$external_prop_getter' : (_this) => _this.kotlinException,
    'kotlin.js.kotlinException_$external_prop_setter' : (_this, v) => _this.kotlinException = v,
    'kotlin.js.JsError_$external_class_instanceof' : (x) => x instanceof Error,
    'kotlin.js.JsString_$external_class_instanceof' : (x) => typeof x === 'string',
    'kotlin.js.JsString_$external_class_get' : () => JsString,
    'kotlin.js.then_$external_fun' : (_this, p0) => _this.then(p0),
    'kotlin.js.__convertKotlinClosureToJsClosure_((Js?)->Js?)' : (f) => getCachedJsObject(f, (p0) => wasmExports['__callFunction_((Js?)->Js?)'](f, p0)),
    'kotlin.js.then_$external_fun_1' : (_this, p0, p1) => _this.then(p0, p1),
    'kotlin.js.__convertKotlinClosureToJsClosure_((Js)->Js?)' : (f) => getCachedJsObject(f, (p0) => wasmExports['__callFunction_((Js)->Js?)'](f, p0)),
    'kotlin.random.initialSeed' : () => ((Math.random() * Math.pow(2, 32)) | 0),
    'kotlin.wasm.internal.getJsClassName' : (jsKlass) => jsKlass.name,
    'kotlin.wasm.internal.instanceOf' : (ref, jsKlass) => ref instanceof jsKlass,
    'kotlin.wasm.internal.getConstructor' : (obj) => obj.constructor,
    'kotlin.time.tryGetPerformance' : () => typeof globalThis !== 'undefined' && typeof globalThis.performance !== 'undefined' ? globalThis.performance : null,
    'kotlin.time.getPerformanceNow' : (performance) => performance.now(),
    'kotlin.time.dateNow' : () => Date.now(),
    'kotlinx.coroutines.tryGetProcess' : () => (typeof(process) !== 'undefined' && typeof(process.nextTick) === 'function') ? process : null,
    'kotlinx.coroutines.tryGetWindow' : () => (typeof(window) !== 'undefined' && window != null && typeof(window.addEventListener) === 'function') ? window : null,
    'kotlinx.coroutines.nextTick_$external_fun' : (_this, p0) => _this.nextTick(p0),
    'kotlinx.coroutines.createScheduleMessagePoster' : (process) => () => Promise.resolve(0).then(process),
    'kotlinx.coroutines.__callJsClosure_(()->Unit)' : (f, ) => f(),
    'kotlinx.coroutines.createRescheduleMessagePoster' : (window) => () => window.postMessage('dispatchCoroutine', '*'),
    'kotlinx.coroutines.subscribeToWindowMessages' : (window, process) => {
        const handler = (event) => {
            if (event.source == window && event.data == 'dispatchCoroutine') {
                event.stopPropagation();
                process();
            }
        }
        window.addEventListener('message', handler, true);
    },
    'kotlinx.coroutines.setTimeout' : (window, handler, timeout) => window.setTimeout(handler, timeout),
    'kotlinx.coroutines.clearTimeout' : (handle) => { if (typeof clearTimeout !== 'undefined') clearTimeout(handle); },
    'kotlinx.coroutines.clearTimeout_$external_fun' : (_this, p0) => _this.clearTimeout(p0),
    'kotlinx.coroutines.setTimeout_$external_fun' : (p0, p1) => setTimeout(p0, p1),
    'kotlinx.coroutines.internal.throwAsyncJsError' : (message, className, stack) => { 
            var error = new Error();
            error.message = message;
            error.name = className;
            error.stack = stack;
            if (typeof globalThis.reportError === 'function') {
                // Up-to-date browsers and some non-browser JS runtimes (Deno, Bun)
                globalThis.reportError(error);
            } else {
                // Fallback for old browsers (pre-2022), Node.js
                setTimeout(function () { throw error }, 0);
            }
         },
    'org.khronos.webgl.byteLength_$external_prop_getter' : (_this) => _this.byteLength,
    'org.khronos.webgl.buffer_$external_prop_getter' : (_this) => _this.buffer,
    'org.w3c.dom.css.cursor_$external_prop_setter' : (_this, v) => _this.cursor = v,
    'org.w3c.dom.css.height_$external_prop_setter' : (_this, v) => _this.height = v,
    'org.w3c.dom.css.left_$external_prop_setter' : (_this, v) => _this.left = v,
    'org.w3c.dom.css.opacity_$external_prop_setter' : (_this, v) => _this.opacity = v,
    'org.w3c.dom.css.outline_$external_prop_setter' : (_this, v) => _this.outline = v,
    'org.w3c.dom.css.position_$external_prop_setter' : (_this, v) => _this.position = v,
    'org.w3c.dom.css.top_$external_prop_setter' : (_this, v) => _this.top = v,
    'org.w3c.dom.css.whiteSpace_$external_prop_setter' : (_this, v) => _this.whiteSpace = v,
    'org.w3c.dom.css.width_$external_prop_setter' : (_this, v) => _this.width = v,
    'org.w3c.dom.css.setProperty_$external_fun' : (_this, p0, p1, p2, isDefault0) => _this.setProperty(p0, p1, isDefault0 ? undefined : p2, ),
    'org.w3c.dom.css.style_$external_prop_getter' : (_this) => _this.style,
    'org.w3c.dom.events.addEventListener_$external_fun' : (_this, p0, p1, p2) => _this.addEventListener(p0, p1, p2),
    'org.w3c.dom.events.__convertKotlinClosureToJsClosure_((Js)->Unit)' : (f) => getCachedJsObject(f, (p0) => wasmExports['__callFunction_((Js)->Unit)'](f, p0)),
    'org.w3c.dom.events.addEventListener_$external_fun_1' : (_this, p0, p1) => _this.addEventListener(p0, p1),
    'org.w3c.dom.events.type_$external_prop_getter' : (_this) => _this.type,
    'org.w3c.dom.events.target_$external_prop_getter' : (_this) => _this.target,
    'org.w3c.dom.events.cancelable_$external_prop_getter' : (_this) => _this.cancelable,
    'org.w3c.dom.events.timeStamp_$external_prop_getter' : (_this) => _this.timeStamp,
    'org.w3c.dom.events.preventDefault_$external_fun' : (_this, ) => _this.preventDefault(),
    'org.w3c.dom.events.Event_$external_class_instanceof' : (x) => x instanceof Event,
    'org.w3c.dom.events.Event_$external_class_get' : () => Event,
    'org.w3c.dom.events.FocusEvent_$external_class_instanceof' : (x) => x instanceof FocusEvent,
    'org.w3c.dom.events.FocusEvent_$external_class_get' : () => FocusEvent,
    'org.w3c.dom.events.clientX_$external_prop_getter' : (_this) => _this.clientX,
    'org.w3c.dom.events.clientY_$external_prop_getter' : (_this) => _this.clientY,
    'org.w3c.dom.events.ctrlKey_$external_prop_getter' : (_this) => _this.ctrlKey,
    'org.w3c.dom.events.shiftKey_$external_prop_getter' : (_this) => _this.shiftKey,
    'org.w3c.dom.events.altKey_$external_prop_getter' : (_this) => _this.altKey,
    'org.w3c.dom.events.metaKey_$external_prop_getter' : (_this) => _this.metaKey,
    'org.w3c.dom.events.button_$external_prop_getter' : (_this) => _this.button,
    'org.w3c.dom.events.buttons_$external_prop_getter' : (_this) => _this.buttons,
    'org.w3c.dom.events.offsetX_$external_prop_getter' : (_this) => _this.offsetX,
    'org.w3c.dom.events.offsetY_$external_prop_getter' : (_this) => _this.offsetY,
    'org.w3c.dom.events.key_$external_prop_getter' : (_this) => _this.key,
    'org.w3c.dom.events.code_$external_prop_getter' : (_this) => _this.code,
    'org.w3c.dom.events.ctrlKey_$external_prop_getter_1' : (_this) => _this.ctrlKey,
    'org.w3c.dom.events.shiftKey_$external_prop_getter_1' : (_this) => _this.shiftKey,
    'org.w3c.dom.events.altKey_$external_prop_getter_1' : (_this) => _this.altKey,
    'org.w3c.dom.events.metaKey_$external_prop_getter_1' : (_this) => _this.metaKey,
    'org.w3c.dom.events.KeyboardEvent_$external_class_instanceof' : (x) => x instanceof KeyboardEvent,
    'org.w3c.dom.events.KeyboardEvent_$external_class_get' : () => KeyboardEvent,
    'org.w3c.dom.events.deltaX_$external_prop_getter' : (_this) => _this.deltaX,
    'org.w3c.dom.events.deltaY_$external_prop_getter' : (_this) => _this.deltaY,
    'org.w3c.dom.events.WheelEvent_$external_class_instanceof' : (x) => x instanceof WheelEvent,
    'org.w3c.dom.events.WheelEvent_$external_class_get' : () => WheelEvent,
    'org.w3c.dom.ShadowRootInit' : (mode) => ({ mode: mode }),
    'org.w3c.dom.AddEventListenerOptions_js_code' : (passive, once, capture) => ({ passive: passive, once: once, capture: capture }),
    'org.w3c.dom.location_$external_prop_getter' : (_this) => _this.location,
    'org.w3c.dom.navigator_$external_prop_getter' : (_this) => _this.navigator,
    'org.w3c.dom.devicePixelRatio_$external_prop_getter' : (_this) => _this.devicePixelRatio,
    'org.w3c.dom.requestAnimationFrame_$external_fun' : (_this, p0) => _this.requestAnimationFrame(p0),
    'org.w3c.dom.__convertKotlinClosureToJsClosure_((Double)->Unit)' : (f) => getCachedJsObject(f, (p0) => wasmExports['__callFunction_((Double)->Unit)'](f, p0)),
    'org.w3c.dom.matchMedia_$external_fun' : (_this, p0) => _this.matchMedia(p0),
    'org.w3c.dom.matches_$external_prop_getter' : (_this) => _this.matches,
    'org.w3c.dom.addListener_$external_fun' : (_this, p0) => _this.addListener(p0),
    'org.w3c.dom.MediaQueryList_$external_class_instanceof' : (x) => x instanceof MediaQueryList,
    'org.w3c.dom.MediaQueryList_$external_class_get' : () => MediaQueryList,
    'org.w3c.dom.protocol_$external_prop_getter' : (_this) => _this.protocol,
    'org.w3c.dom.dropEffect_$external_prop_setter' : (_this, v) => _this.dropEffect = v,
    'org.w3c.dom.setDragImage_$external_fun' : (_this, p0, p1, p2) => _this.setDragImage(p0, p1, p2),
    'org.w3c.dom.setData_$external_fun' : (_this, p0, p1) => _this.setData(p0, p1),
    'org.w3c.dom.tabIndex_$external_prop_setter' : (_this, v) => _this.tabIndex = v,
    'org.w3c.dom.innerText_$external_prop_setter' : (_this, v) => _this.innerText = v,
    'org.w3c.dom.click_$external_fun' : (_this, ) => _this.click(),
    'org.w3c.dom.HTMLElement_$external_class_instanceof' : (x) => x instanceof HTMLElement,
    'org.w3c.dom.HTMLElement_$external_class_get' : () => HTMLElement,
    'org.w3c.dom.userAgent_$external_prop_getter' : (_this) => _this.userAgent,
    'org.w3c.dom.language_$external_prop_getter' : (_this) => _this.language,
    'org.w3c.dom.caches_$external_prop_getter' : (_this) => _this.caches,
    'org.w3c.dom.fetch_$external_fun' : (_this, p0, p1, isDefault0) => _this.fetch(p0, isDefault0 ? undefined : p1, ),
    'org.w3c.dom.body_$external_prop_getter' : (_this) => _this.body,
    'org.w3c.dom.activeElement_$external_prop_getter' : (_this) => _this.activeElement,
    'org.w3c.dom.createElement_$external_fun' : (_this, p0, p1, isDefault0) => _this.createElement(p0, isDefault0 ? undefined : p1, ),
    'org.w3c.dom.hasFocus_$external_fun' : (_this, ) => _this.hasFocus(),
    'org.w3c.dom.id_$external_prop_setter' : (_this, v) => _this.id = v,
    'org.w3c.dom.shadowRoot_$external_prop_getter' : (_this) => _this.shadowRoot,
    'org.w3c.dom.clientWidth_$external_prop_getter' : (_this) => _this.clientWidth,
    'org.w3c.dom.clientHeight_$external_prop_getter' : (_this) => _this.clientHeight,
    'org.w3c.dom.setAttribute_$external_fun' : (_this, p0, p1) => _this.setAttribute(p0, p1),
    'org.w3c.dom.attachShadow_$external_fun' : (_this, p0) => _this.attachShadow(p0),
    'org.w3c.dom.getBoundingClientRect_$external_fun' : (_this, ) => _this.getBoundingClientRect(),
    'org.w3c.dom.dataTransfer_$external_prop_getter' : (_this) => _this.dataTransfer,
    'org.w3c.dom.DragEvent_$external_class_instanceof' : (x) => x instanceof DragEvent,
    'org.w3c.dom.DragEvent_$external_class_get' : () => DragEvent,
    'org.w3c.dom.isConnected_$external_prop_getter' : (_this) => _this.isConnected,
    'org.w3c.dom.firstChild_$external_prop_getter' : (_this) => _this.firstChild,
    'org.w3c.dom.textContent_$external_prop_setter' : (_this, v) => _this.textContent = v,
    'org.w3c.dom.hasChildNodes_$external_fun' : (_this, ) => _this.hasChildNodes(),
    'org.w3c.dom.appendChild_$external_fun' : (_this, p0) => _this.appendChild(p0),
    'org.w3c.dom.removeChild_$external_fun' : (_this, p0) => _this.removeChild(p0),
    'org.w3c.dom.top_$external_prop_getter' : (_this) => _this.top,
    'org.w3c.dom.left_$external_prop_getter' : (_this) => _this.left,
    'org.w3c.dom.remove_$external_fun' : (_this, ) => _this.remove(),
    'org.w3c.dom.Companion_$external_object_getInstance' : () => ({}),
    'org.w3c.dom.HTMLDivElement_$external_class_instanceof' : (x) => x instanceof HTMLDivElement,
    'org.w3c.dom.HTMLDivElement_$external_class_get' : () => HTMLDivElement,
    'org.w3c.dom.width_$external_prop_getter' : (_this) => _this.width,
    'org.w3c.dom.width_$external_prop_setter' : (_this, v) => _this.width = v,
    'org.w3c.dom.height_$external_prop_getter' : (_this) => _this.height,
    'org.w3c.dom.height_$external_prop_setter' : (_this, v) => _this.height = v,
    'org.w3c.dom.HTMLCanvasElement_$external_class_instanceof' : (x) => x instanceof HTMLCanvasElement,
    'org.w3c.dom.HTMLCanvasElement_$external_class_get' : () => HTMLCanvasElement,
    'org.w3c.dom.HTMLTextAreaElement_$external_class_instanceof' : (x) => x instanceof HTMLTextAreaElement,
    'org.w3c.dom.HTMLTextAreaElement_$external_class_get' : () => HTMLTextAreaElement,
    'org.w3c.dom.TouchEvent_$external_class_instanceof' : (x) => x instanceof TouchEvent,
    'org.w3c.dom.TouchEvent_$external_class_get' : () => TouchEvent,
    'org.w3c.dom.matches_$external_prop_getter_1' : (_this) => _this.matches,
    'org.w3c.dom.MediaQueryListEvent_$external_class_instanceof' : (x) => x instanceof MediaQueryListEvent,
    'org.w3c.dom.MediaQueryListEvent_$external_class_get' : () => MediaQueryListEvent,
    'org.w3c.dom.pointerevents.pointerId_$external_prop_getter' : (_this) => _this.pointerId,
    'org.w3c.dom.pointerevents.pressure_$external_prop_getter' : (_this) => _this.pressure,
    'org.w3c.dom.pointerevents.PointerEvent_$external_class_instanceof' : (x) => x instanceof PointerEvent,
    'org.w3c.dom.pointerevents.PointerEvent_$external_class_get' : () => PointerEvent,
    'org.w3c.fetch.ok_$external_prop_getter' : (_this) => _this.ok,
    'org.w3c.fetch.clone_$external_fun' : (_this, ) => _this.clone(),
    'org.w3c.fetch.blob_$external_fun' : (_this, ) => _this.blob(),
    'org.w3c.performance.now_$external_fun' : (_this, ) => _this.now(),
    'org.w3c.performance.performance_$external_prop_getter' : (_this) => _this.performance,
    'org.w3c.workers.CacheQueryOptions_js_code' : (ignoreSearch, ignoreMethod, ignoreVary, cacheName) => ({ ignoreSearch: ignoreSearch, ignoreMethod: ignoreMethod, ignoreVary: ignoreVary, cacheName: cacheName }),
    'org.w3c.workers.match_$external_fun' : (_this, p0, p1, isDefault0) => _this.match(p0, isDefault0 ? undefined : p1, ),
    'org.w3c.workers.put_$external_fun' : (_this, p0, p1) => _this.put(p0, p1),
    'org.w3c.workers.open_$external_fun' : (_this, p0) => _this.open(p0),
    'org.w3c.workers.delete_$external_fun' : (_this, p0) => _this.delete(p0),
    'org.w3c.xhr.XMLHttpRequest_$external_fun' : () => new XMLHttpRequest(),
    'org.w3c.xhr.status_$external_prop_getter' : (_this) => _this.status,
    'org.w3c.xhr.open_$external_fun' : (_this, p0, p1, p2, p3, p4, isDefault0, isDefault1) => _this.open(p0, p1, p2, isDefault0 ? undefined : p3, isDefault1 ? undefined : p4, ),
    'org.w3c.xhr.send_$external_fun' : (_this, ) => _this.send(),
    'org.w3c.xhr.overrideMimeType_$external_fun' : (_this, p0) => _this.overrideMimeType(p0),
    'kotlinx.browser.window_$external_prop_getter' : () => window,
    'kotlinx.browser.document_$external_prop_getter' : () => document,
    'androidx.compose.runtime.internal.weakMap_js_code' : () => (new WeakMap()),
    'androidx.compose.runtime.internal.set_$external_fun' : (_this, p0, p1) => _this.set(p0, p1),
    'androidx.compose.runtime.internal.get_$external_fun' : (_this, p0) => _this.get(p0),
    'androidx.compose.runtime.internal.WeakRef_$external_fun' : (p0) => new WeakRef(p0),
    'androidx.compose.runtime.internal.deref_$external_fun' : (_this, ) => _this.deref(),
    'org.jetbrains.skiko.GL_$external_prop_getter' : () => _ref_Li9za2lrby5tanM_.GL,
    'org.jetbrains.skia.impl.FinalizationRegistry_$external_fun' : (p0) => new FinalizationRegistry(p0),
    'org.jetbrains.skia.impl.register_$external_fun' : (_this, p0, p1, p2) => _this.register(p0, p1, p2),
    'org.jetbrains.skia.impl.unregister_$external_fun' : (_this, p0) => _this.unregister(p0),
    'org.jetbrains.skia.impl._releaseLocalCallbackScope_$external_fun' : () => _ref_Li9za2lrby5tanM_c2tpa29BcGk._releaseLocalCallbackScope(),
    'org.jetbrains.skiko.wasm.createDefaultContextAttributes' : () => {
        return {
            alpha: 1,
            depth: 1,
            stencil: 8,
            antialias: 0,
            premultipliedAlpha: 1,
            preserveDrawingBuffer: 0,
            preferLowPowerToHighPerformance: 0,
            failIfMajorPerformanceCaveat: 0,
            enableExtensionsByDefault: 1,
            explicitSwapControl: 0,
            renderViaOffscreenBackBuffer: 0,
            majorVersion: 2,
        }
    }
    ,
    'org.jetbrains.skiko.wasm.awaitSkiko_$external_prop_getter' : () => _ref_Li9za2lrby5tanM_.awaitSkiko,
    'org.jetbrains.skiko.createContext_$external_fun' : (_this, p0, p1) => _this.createContext(p0, p1),
    'org.jetbrains.skiko.makeContextCurrent_$external_fun' : (_this, p0) => _this.makeContextCurrent(p0),
    'org.jetbrains.skiko.getNavigatorInfo' : () => navigator.userAgentData ? navigator.userAgentData.platform : navigator.platform,
    'androidx.compose.ui.text.intl.getUserPreferredLanguagesAsArray' : () => window.navigator.languages,
    'androidx.compose.ui.text.FinalizationRegistry_$external_fun' : (p0) => new FinalizationRegistry(p0),
    'androidx.compose.ui.text.register_$external_fun' : (_this, p0, p1) => _this.register(p0, p1),
    'androidx.compose.ui.text.WeakRef_$external_fun' : (p0) => new WeakRef(p0),
    'androidx.compose.ui.text.deref_$external_fun' : (_this, ) => _this.deref(),
    'androidx.compose.ui.text.intl.parseLanguageTagToIntlLocale' : (languageTag) => new Intl.Locale(languageTag),
    'androidx.compose.ui.text.intl._language_$external_prop_getter' : (_this) => _this.language,
    'androidx.compose.ui.text.intl._region_$external_prop_getter' : (_this) => _this.region,
    'androidx.compose.ui.text.intl._baseName_$external_prop_getter' : (_this) => _this.baseName,
    'androidx.compose.ui.internal.weakMap_js_code' : () => (new WeakMap()),
    'androidx.compose.ui.internal.set_$external_fun' : (_this, p0, p1) => _this.set(p0, p1),
    'androidx.compose.ui.internal.get_$external_fun' : (_this, p0) => _this.get(p0),
    'androidx.compose.ui.platform.warn' : (text) => { console.warn(text) },
    'androidx.compose.ui.events.withSignal' : (signal) => ({signal: signal}),
    'androidx.compose.ui.events.withSignalAndPassive' : (signal, passive) => ({signal: signal, passive: passive}),
    'androidx.compose.ui.events.AbortController_$external_fun' : () => new AbortController(),
    'androidx.compose.ui.events.signal_$external_prop_getter' : (_this) => _this.signal,
    'androidx.compose.ui.internal.focusExt' : (element, _preventScroll) => element.focus({ preventScroll: _preventScroll }),
    'androidx.compose.ui.node.WeakRef_$external_fun' : (p0) => new WeakRef(p0),
    'androidx.compose.ui.node.deref_$external_fun' : (_this, ) => _this.deref(),
    'androidx.compose.ui.platform.getW3CClipboard' : () => window.navigator.clipboard,
    'androidx.compose.ui.platform.isSecureContext' : () => window.isSecureContext === true,
    'androidx.compose.ui.platform.isFullClipboardApiSupported' : () => Boolean(
            window.navigator.clipboard && 
            window.navigator.clipboard.write && 
            window.navigator.clipboard.read && 
            typeof(ClipboardItem) !== 'undefined'
            )
        ,
    'androidx.compose.ui.platform.isFallbackWriteTextApiAvailable' : () => Boolean(window.navigator.clipboard && window.navigator.clipboard.writeText),
    'androidx.compose.ui.platform.W3CTemporaryClipboard_$external_class_instanceof' : (x) => x instanceof Clipboard,
    'androidx.compose.ui.platform.W3CTemporaryClipboard_$external_class_get' : () => Clipboard,
    'androidx.compose.ui.platform.accessibility.removeAllChildrenOf' : (element) => { element.replaceChildren() },
    'androidx.compose.ui.platform.accessibility.setA11YAriaRole' : (element, ariaRoleId) => { 
            var roleValue = "";
            switch (ariaRoleId) {
                case 0: // Role.Button
                    roleValue = "button";
                    break;
                case 1: // Role.Checkbox
                    roleValue = "checkbox";
                    break;
                case 2: // Role.Switch
                    roleValue = "switch";
                    break;
                case 3: // Role.RadioButton
                    roleValue = "radio";
                    break;
                case 4: // Role.Tab
                    roleValue = "tab";
                    break;
                case 5: // Role.Image
                    roleValue = "img";
                    break;
                case 6: // Role.DropdownList
                    roleValue = "menu";
                    break;
                case 7: // heading https://developer.mozilla.org/en-US/docs/Web/Accessibility/ARIA/Reference/Roles/heading_role
                    roleValue = "heading";
                    break;
                case 8: // https://developer.mozilla.org/en-US/docs/Web/Accessibility/ARIA/Reference/Roles/textbox_role
                    roleValue = "textbox";
                    break;
                case 9: // https://developer.mozilla.org/en-US/docs/Web/Accessibility/ARIA/Reference/Roles/list_role
                    roleValue = "list";
                    break;
                case 10: // https://developer.mozilla.org/en-US/docs/Web/Accessibility/ARIA/Reference/Roles/grid_role
                    roleValue = "grid";
                    break;
                default:
                    break;
            }
            if (roleValue.length > 0) { 
                element.setAttribute("role", roleValue);
            } else {
                element.removeAttribute("role");
            }
         },
    'androidx.compose.ui.platform.accessibility.setSizeAndPosition' : (element, left, top, width, height) => { 
           element.style.left = "" + left + "px";
           element.style.top = "" + top + "px";
           element.style.width = "" + width + "px";
           element.style.height = "" + height + "px";
         },
    'androidx.compose.ui.window.documentIsVisible' : () => document.visibilityState === 'visible',
    'androidx.compose.ui.window.isMouseEvent' : (event) => event.pointerType === 'mouse',
    'androidx.compose.ui.window.setPointerCapture' : (target, pointerId) => { try { target.setPointerCapture(pointerId) } catch (e) {} },
    'androidx.compose.ui.window.getCoalescedEvents' : (pointerEvent) => pointerEvent.getCoalescedEvents ? pointerEvent.getCoalescedEvents() : [],
    'androidx.compose.ui.window.getPointerEventCode' : (event) => {
            switch (event.type) {
              case 'pointerdown':
                return 1; // PointerEventType.Press
              case 'pointerup':
              case 'pointercancel':
                return 2; // PointerEventType.Release
              case 'pointermove':
                return 3; // PointerEventType.Move
              case 'pointerenter':
                return 4; //PointerEventType.Enter
              case 'pointerleave':
                return 5; //PointerEventType.Exit
              default:
                return 0; // PointerEventType.Unknown
            } 
        },
    'androidx.compose.ui.window.activeElement_$external_prop_getter' : (_this) => _this.activeElement,
    'androidx.compose.ui.window.isMatchMediaSupported' : () => window.matchMedia != undefined,
    'org.jetbrains.compose.resources.Locale_$external_fun' : (p0) => new Intl.Locale(p0),
    'org.jetbrains.compose.resources.language_$external_prop_getter' : (_this) => _this.language,
    'org.jetbrains.compose.resources.region_$external_prop_getter' : (_this) => _this.region,
    'org.jetbrains.compose.resources.isInTestEnvironment' : () => window.composeResourcesTesting == true,
    'org.jetbrains.compose.resources.requestResponseAsByteArray' : (req) =>  {
            var text = req.responseText;
            var int8Arr = new Int8Array(text.length);
            for (var i = 0; i < text.length; i++) {
                int8Arr[i] = text.charCodeAt(i) & 0xFF;
            }
            return int8Arr;
        },
    'org.jetbrains.compose.resources.jsExportBlobAsArrayBuffer' : (blob) => blob.arrayBuffer(),
    'org.jetbrains.compose.resources.copyArrayBufferToWasmMemory' : (ab, ptr) => {
          const data = new Uint8Array(ab);
          new Uint8Array(wasmExports.memory.buffer).set(data, ptr);
    },
    'org.jetbrains.compose.resources.supportsCacheApi' : () => Boolean(window.caches),
    'dev.kampr.terminal.spike.consoleLog' : (s) => { console.log(s) },
    'dev.kampr.terminal.spike.postLine' : (s) => { 
            try {
                fetch('/bench', { method: 'POST', headers: { 'Content-Type': 'text/plain' }, body: s });
            } catch (e) {}
             },
    'dev.kampr.terminal.spike.probeBackend' : () => 
        (function () {
            var out = [];
            var seen = [];
            function walk(root, depth) {
                if (!root || depth > 6) return;
                var kids = root.children || [];
                for (var i = 0; i < kids.length; i++) {
                    var el = kids[i];
                    if (el.tagName === 'CANVAS') seen.push(el);
                    if (el.shadowRoot) walk(el.shadowRoot, depth + 1);
                    walk(el, depth + 1);
                }
            }
            walk(document.body, 0);
            out.push('canvases=' + seen.length);
            out.push('bodyChildren=' + Array.prototype.map.call(document.body.children, function (e) {
                return e.tagName.toLowerCase() + (e.id ? '#' + e.id : '');
            }).join(','));
            for (var i = 0; i < seen.length; i++) {
                var c = seen[i];
                var kind = 'no-context';
                var extra = '';
                try {
                    if (c.getContext('webgpu')) { kind = 'webgpu'; }
                } catch (e) {}
                try {
                    if (kind === 'no-context' && c.getContext('webgl2')) {
                        kind = 'webgl2';
                        var gl = c.getContext('webgl2');
                        var dbg = gl.getExtension('WEBGL_debug_renderer_info');
                        extra = ' renderer=' + (dbg ? gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER));
                    }
                } catch (e) {}
                try {
                    if (kind === 'no-context' && c.getContext('webgl')) {
                        kind = 'webgl1';
                        var g1 = c.getContext('webgl');
                        var d1 = g1.getExtension('WEBGL_debug_renderer_info');
                        extra = ' renderer=' + (d1 ? g1.getParameter(d1.UNMASKED_RENDERER_WEBGL) : g1.getParameter(g1.RENDERER));
                    }
                } catch (e) {}
                try {
                    if (kind === 'no-context' && c.getContext('2d')) { kind = 'canvas2d'; }
                } catch (e) {}
                out.push('canvas' + i + '=' + kind + extra + ' css=' + c.width + 'x' + c.height);
            }
            out.push('webgpu_api=' + (navigator.gpu ? 'present' : 'absent'));
            out.push('dpr=' + window.devicePixelRatio);
            return out.join(' | ');
        })()

}

const StringConstantsProxy = new Proxy({}, {
  get(_, prop) { return prop; }
});

export { wasmTag as __TAG };

export const importObject = {
    js_code,
    intrinsics: {
        tag: wasmTag
    },
    "'": StringConstantsProxy,
    'wasm:js-string': d2FzbTpqcy1zdHJpbmc,
    './skiko.mjs': Li9za2lrby5tanM,
};
    