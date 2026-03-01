use sir_debug_backend::ir_to_bytecode;
use sir_optimizations::Optimizer;
use sir_parser::{EmitConfig, ParseIrError, ParseIrErrorKind, Span, parse_ir};
use std::panic::{AssertUnwindSafe, catch_unwind};

const ABI_VERSION: u32 = 1;
const BYTECODE_VERSION: u32 = 1;
const DEFAULT_INIT_NAME: &str = "init";
const DEFAULT_MAIN_NAME: &str = "main";

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SenseicStatus {
    Ok = 0,
    InvalidArgument = 1,
    ParseError = 2,
    EmitError = 3,
    IllegalIrError = 4,
    BackendError = 5,
    Panic = 6,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SenseicDiagnosticKind {
    Parse = 1,
    Emit = 2,
    IllegalIr = 3,
    Backend = 4,
    Internal = 5,
    Panic = 6,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SenseicByteSlice {
    pub ptr: *const u8,
    pub len: usize,
}

impl SenseicByteSlice {
    pub const fn empty() -> Self {
        Self { ptr: std::ptr::null(), len: 0 }
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        if bytes.is_empty() {
            Self::empty()
        } else {
            Self { ptr: bytes.as_ptr(), len: bytes.len() }
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SenseicCompileOptions {
    pub init_only: u8,
    pub init_name: SenseicByteSlice,
    pub main_name: SenseicByteSlice,
    pub optimize_passes: SenseicByteSlice,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SenseicDiagnostic {
    pub kind: SenseicDiagnosticKind,
    pub span_start: u32,
    pub span_end: u32,
    pub message: SenseicByteSlice,
}

#[repr(C)]
pub struct SenseicCompileResult {
    _private: [u8; 0],
}

struct DiagnosticOwned {
    kind: SenseicDiagnosticKind,
    span_start: u32,
    span_end: u32,
}

struct CompileResultOwned {
    status: SenseicStatus,
    message: Vec<u8>,
    bytecode: Vec<u8>,
    diagnostics: Vec<DiagnosticOwned>,
}

impl CompileResultOwned {
    fn ok(bytecode: Vec<u8>) -> Self {
        Self { status: SenseicStatus::Ok, message: Vec::new(), bytecode, diagnostics: Vec::new() }
    }

    fn invalid_argument(message: impl Into<String>) -> Self {
        Self::single_error(
            SenseicStatus::InvalidArgument,
            SenseicDiagnosticKind::Internal,
            message.into(),
            &[],
        )
    }

    fn panic_boundary() -> Self {
        Self::single_error(
            SenseicStatus::Panic,
            SenseicDiagnosticKind::Panic,
            "panic across FFI boundary".to_string(),
            &[],
        )
    }

    fn backend_error(message: impl Into<String>) -> Self {
        Self::single_error(
            SenseicStatus::BackendError,
            SenseicDiagnosticKind::Backend,
            message.into(),
            &[],
        )
    }

    fn parse_error(err: ParseIrError) -> Self {
        let status = match err.kind {
            ParseIrErrorKind::Parse => SenseicStatus::ParseError,
            ParseIrErrorKind::Emit => SenseicStatus::EmitError,
            ParseIrErrorKind::IllegalIr => SenseicStatus::IllegalIrError,
        };

        let diagnostic_kind = match err.kind {
            ParseIrErrorKind::Parse => SenseicDiagnosticKind::Parse,
            ParseIrErrorKind::Emit => SenseicDiagnosticKind::Emit,
            ParseIrErrorKind::IllegalIr => SenseicDiagnosticKind::IllegalIr,
        };

        Self::single_error(status, diagnostic_kind, err.message, &err.spans)
    }

    fn single_error(
        status: SenseicStatus,
        diagnostic_kind: SenseicDiagnosticKind,
        message: String,
        spans: &[Span],
    ) -> Self {
        let message_bytes = message.into_bytes();
        let diagnostics = if spans.is_empty() {
            vec![DiagnosticOwned { kind: diagnostic_kind, span_start: 0, span_end: 0 }]
        } else {
            spans
                .iter()
                .map(|span| DiagnosticOwned {
                    kind: diagnostic_kind,
                    span_start: saturating_u32(span.start),
                    span_end: saturating_u32(span.end),
                })
                .collect()
        };
        Self { status, message: message_bytes, bytecode: Vec::new(), diagnostics }
    }
}

#[derive(Debug, Clone)]
struct CompileOptionsOwned {
    init_only: bool,
    init_name: String,
    main_name: String,
    optimize_passes: Option<String>,
}

impl Default for CompileOptionsOwned {
    fn default() -> Self {
        Self {
            init_only: false,
            init_name: DEFAULT_INIT_NAME.to_string(),
            main_name: DEFAULT_MAIN_NAME.to_string(),
            optimize_passes: None,
        }
    }
}

fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

unsafe fn byte_slice_to_bytes<'a>(
    slice: SenseicByteSlice,
    field_name: &str,
) -> Result<&'a [u8], String> {
    if slice.len == 0 {
        return Ok(&[]);
    }
    if slice.ptr.is_null() {
        return Err(format!("{field_name} pointer is null while len is {}", slice.len));
    }
    Ok(unsafe { std::slice::from_raw_parts(slice.ptr, slice.len) })
}

unsafe fn decode_optional_utf8(
    slice: SenseicByteSlice,
    field_name: &str,
) -> Result<Option<String>, String> {
    if slice.len == 0 {
        return Ok(None);
    }
    let bytes = unsafe { byte_slice_to_bytes(slice, field_name)? };
    let decoded = std::str::from_utf8(bytes)
        .map_err(|err| format!("{field_name} is not valid UTF-8: {err}"))?;
    Ok(Some(decoded.to_string()))
}

unsafe fn decode_required_utf8(
    slice: SenseicByteSlice,
    field_name: &str,
) -> Result<String, String> {
    let bytes = unsafe { byte_slice_to_bytes(slice, field_name)? };
    let decoded = std::str::from_utf8(bytes)
        .map_err(|err| format!("{field_name} is not valid UTF-8: {err}"))?;
    Ok(decoded.to_string())
}

fn validate_optimize_passes(passes: &str) -> Result<(), String> {
    for c in passes.chars() {
        if !matches!(c, 's' | 'c' | 'u' | 'd') {
            return Err(format!("invalid optimization pass '{c}', valid passes: s, c, u, d"));
        }
    }
    Ok(())
}

unsafe fn decode_options(
    options: *const SenseicCompileOptions,
) -> Result<CompileOptionsOwned, String> {
    if options.is_null() {
        return Ok(CompileOptionsOwned::default());
    }

    let options = unsafe { &*options };
    let mut owned =
        CompileOptionsOwned { init_only: options.init_only != 0, ..CompileOptionsOwned::default() };

    if let Some(init_name) = unsafe { decode_optional_utf8(options.init_name, "init_name")? } {
        owned.init_name = init_name;
    }
    if let Some(main_name) = unsafe { decode_optional_utf8(options.main_name, "main_name")? } {
        owned.main_name = main_name;
    }
    if let Some(passes) =
        unsafe { decode_optional_utf8(options.optimize_passes, "optimize_passes")? }
    {
        validate_optimize_passes(&passes)?;
        owned.optimize_passes = Some(passes);
    }
    Ok(owned)
}

fn compile_source(source: &str, options: &CompileOptionsOwned) -> CompileResultOwned {
    let config = if options.init_only {
        EmitConfig::init_only_with_name(&options.init_name)
    } else {
        EmitConfig::new(&options.init_name, &options.main_name)
    };

    let mut program = match parse_ir(source, config) {
        Ok(program) => program,
        Err(err) => return CompileResultOwned::parse_error(err),
    };

    if let Some(passes) = options.optimize_passes.as_deref() {
        let mut optimizer = Optimizer::new(program);
        optimizer.run_passes(passes);
        program = optimizer.finish();
    }

    let mut bytecode = Vec::with_capacity(0x6000);
    if let Err(err) = ir_to_bytecode(&program, &mut bytecode) {
        return CompileResultOwned::backend_error(err.to_string());
    }

    CompileResultOwned::ok(bytecode)
}

fn into_result_ptr(result: CompileResultOwned) -> *mut SenseicCompileResult {
    Box::into_raw(Box::new(result)).cast()
}

unsafe fn as_result_ref<'a>(result: *const SenseicCompileResult) -> Option<&'a CompileResultOwned> {
    if result.is_null() {
        return None;
    }
    unsafe { result.cast::<CompileResultOwned>().as_ref() }
}

fn catch_or<T>(fallback: T, body: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(value) => value,
        Err(_) => fallback,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn senseic_abi_version() -> u32 {
    catch_or(ABI_VERSION, || ABI_VERSION)
}

#[unsafe(no_mangle)]
pub extern "C" fn senseic_bytecode_version() -> u32 {
    catch_or(BYTECODE_VERSION, || BYTECODE_VERSION)
}

#[unsafe(no_mangle)]
pub extern "C" fn senseic_compiler_version() -> SenseicByteSlice {
    catch_or(SenseicByteSlice::empty(), || {
        SenseicByteSlice::from_bytes(env!("CARGO_PKG_VERSION").as_bytes())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn senseic_compile_options_default() -> SenseicCompileOptions {
    catch_or(
        SenseicCompileOptions {
            init_only: 0,
            init_name: SenseicByteSlice::empty(),
            main_name: SenseicByteSlice::empty(),
            optimize_passes: SenseicByteSlice::empty(),
        },
        || SenseicCompileOptions {
            init_only: 0,
            init_name: SenseicByteSlice::empty(),
            main_name: SenseicByteSlice::empty(),
            optimize_passes: SenseicByteSlice::empty(),
        },
    )
}

#[unsafe(no_mangle)]
pub extern "C" fn senseic_compile_sir_to_bytecode(
    source: SenseicByteSlice,
    options: *const SenseicCompileOptions,
) -> *mut SenseicCompileResult {
    match catch_unwind(AssertUnwindSafe(|| {
        let source = match unsafe { decode_required_utf8(source, "source") } {
            Ok(source) => source,
            Err(err) => return into_result_ptr(CompileResultOwned::invalid_argument(err)),
        };
        let options = match unsafe { decode_options(options) } {
            Ok(options) => options,
            Err(err) => return into_result_ptr(CompileResultOwned::invalid_argument(err)),
        };
        into_result_ptr(compile_source(&source, &options))
    })) {
        Ok(result) => result,
        Err(_) => into_result_ptr(CompileResultOwned::panic_boundary()),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn senseic_compile_result_free(result: *mut SenseicCompileResult) {
    catch_or((), || {
        if result.is_null() {
            return;
        }
        unsafe {
            drop(Box::from_raw(result.cast::<CompileResultOwned>()));
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn senseic_compile_result_status(
    result: *const SenseicCompileResult,
) -> SenseicStatus {
    catch_or(SenseicStatus::Panic, || {
        unsafe { as_result_ref(result) }
            .map(|result| result.status)
            .unwrap_or(SenseicStatus::InvalidArgument)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn senseic_compile_result_message(
    result: *const SenseicCompileResult,
) -> SenseicByteSlice {
    catch_or(SenseicByteSlice::empty(), || {
        unsafe { as_result_ref(result) }
            .map(|result| SenseicByteSlice::from_bytes(&result.message))
            .unwrap_or_else(SenseicByteSlice::empty)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn senseic_compile_result_bytecode(
    result: *const SenseicCompileResult,
) -> SenseicByteSlice {
    catch_or(SenseicByteSlice::empty(), || {
        unsafe { as_result_ref(result) }
            .map(|result| SenseicByteSlice::from_bytes(&result.bytecode))
            .unwrap_or_else(SenseicByteSlice::empty)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn senseic_compile_result_diagnostic_count(
    result: *const SenseicCompileResult,
) -> usize {
    catch_or(0, || unsafe { as_result_ref(result) }.map_or(0, |result| result.diagnostics.len()))
}

#[unsafe(no_mangle)]
pub extern "C" fn senseic_compile_result_diagnostic_get(
    result: *const SenseicCompileResult,
    index: usize,
    out_diagnostic: *mut SenseicDiagnostic,
) -> u8 {
    catch_or(0, || {
        if out_diagnostic.is_null() {
            return 0;
        }
        let Some(result) = (unsafe { as_result_ref(result) }) else {
            return 0;
        };
        let Some(diagnostic) = result.diagnostics.get(index) else {
            return 0;
        };
        unsafe {
            out_diagnostic.write(SenseicDiagnostic {
                kind: diagnostic.kind,
                span_start: diagnostic.span_start,
                span_end: diagnostic.span_end,
                message: SenseicByteSlice::from_bytes(&result.message),
            });
        }
        1
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(source: &[u8], options: *const SenseicCompileOptions) -> *mut SenseicCompileResult {
        senseic_compile_sir_to_bytecode(SenseicByteSlice::from_bytes(source), options)
    }

    unsafe fn read_utf8(slice: SenseicByteSlice) -> String {
        if slice.len == 0 {
            return String::new();
        }
        let bytes = unsafe { std::slice::from_raw_parts(slice.ptr, slice.len) };
        std::str::from_utf8(bytes).expect("ffi message should be UTF-8").to_string()
    }

    #[test]
    fn compile_success_returns_bytecode() {
        let source = br#"
            fn init:
                entry {
                    stop
                }
            fn main:
                entry {
                    stop
                }
        "#;

        let result = compile(source, std::ptr::null());
        assert_eq!(senseic_compile_result_status(result), SenseicStatus::Ok);
        let bytecode = senseic_compile_result_bytecode(result);
        assert!(bytecode.len > 0);
        assert_eq!(senseic_compile_result_diagnostic_count(result), 0);
        senseic_compile_result_free(result);
    }

    #[test]
    fn compile_parse_error_returns_diagnostic() {
        let source = br#"
            fn init:
                entry {
                    stop
                }
            data
                bytes
                -0x01
        "#;

        let result = compile(source, std::ptr::null());
        assert_eq!(senseic_compile_result_status(result), SenseicStatus::ParseError);
        assert!(senseic_compile_result_diagnostic_count(result) > 0);
        let mut diagnostic = SenseicDiagnostic {
            kind: SenseicDiagnosticKind::Internal,
            span_start: 0,
            span_end: 0,
            message: SenseicByteSlice::empty(),
        };
        assert_eq!(senseic_compile_result_diagnostic_get(result, 0, &mut diagnostic as *mut _), 1);
        assert_eq!(diagnostic.kind, SenseicDiagnosticKind::Parse);
        let message = unsafe { read_utf8(diagnostic.message) };
        assert!(message.contains("non-negative hex literals"));
        senseic_compile_result_free(result);
    }

    #[test]
    fn invalid_utf8_source_is_invalid_argument() {
        let result = compile(&[0xff], std::ptr::null());
        assert_eq!(senseic_compile_result_status(result), SenseicStatus::InvalidArgument);
        let message = unsafe { read_utf8(senseic_compile_result_message(result)) };
        assert!(message.contains("source is not valid UTF-8"));
        senseic_compile_result_free(result);
    }

    #[test]
    fn invalid_optimization_pass_is_invalid_argument() {
        let options = SenseicCompileOptions {
            init_only: 1,
            init_name: SenseicByteSlice::empty(),
            main_name: SenseicByteSlice::empty(),
            optimize_passes: SenseicByteSlice::from_bytes(b"x"),
        };
        let source = br#"
            fn init:
                entry {
                    stop
                }
        "#;
        let result = compile(source, &options);
        assert_eq!(senseic_compile_result_status(result), SenseicStatus::InvalidArgument);
        let message = unsafe { read_utf8(senseic_compile_result_message(result)) };
        assert!(message.contains("invalid optimization pass"));
        senseic_compile_result_free(result);
    }

    #[test]
    fn null_handle_accessors_return_safe_defaults() {
        assert_eq!(senseic_compile_result_status(std::ptr::null()), SenseicStatus::InvalidArgument);
        assert_eq!(senseic_compile_result_message(std::ptr::null()).len, 0);
        assert_eq!(senseic_compile_result_bytecode(std::ptr::null()).len, 0);
        assert_eq!(senseic_compile_result_diagnostic_count(std::ptr::null()), 0);

        let mut diagnostic = SenseicDiagnostic {
            kind: SenseicDiagnosticKind::Internal,
            span_start: 0,
            span_end: 0,
            message: SenseicByteSlice::empty(),
        };
        assert_eq!(
            senseic_compile_result_diagnostic_get(std::ptr::null(), 0, &mut diagnostic as *mut _),
            0
        );

        senseic_compile_result_free(std::ptr::null_mut());
    }

    #[test]
    fn version_functions_return_valid_values() {
        assert_eq!(senseic_abi_version(), ABI_VERSION);
        assert_eq!(senseic_bytecode_version(), BYTECODE_VERSION);
        let version = senseic_compiler_version();
        assert!(version.len > 0);
        let version_str = unsafe { read_utf8(version) };
        assert!(version_str.chars().all(|c| c.is_ascii_digit() || c == '.'));
    }

    #[test]
    fn init_only_compile_succeeds() {
        let options = SenseicCompileOptions {
            init_only: 1,
            init_name: SenseicByteSlice::empty(),
            main_name: SenseicByteSlice::empty(),
            optimize_passes: SenseicByteSlice::empty(),
        };
        let source = br#"
            fn init:
                entry {
                    stop
                }
        "#;
        let result = compile(source, &options);
        assert_eq!(senseic_compile_result_status(result), SenseicStatus::Ok);
        assert!(senseic_compile_result_bytecode(result).len > 0);
        senseic_compile_result_free(result);
    }

    #[test]
    fn compile_with_optimization_passes() {
        let options = SenseicCompileOptions {
            init_only: 0,
            init_name: SenseicByteSlice::empty(),
            main_name: SenseicByteSlice::empty(),
            optimize_passes: SenseicByteSlice::from_bytes(b"csud"),
        };
        let source = br#"
            fn init:
                entry {
                    stop
                }
            fn main:
                entry {
                    c0 = const 0
                    c32 = const 32
                    a = calldataload c0
                    b = calldataload c32
                    c = add a b
                    return c0 c32
                }
        "#;
        let result = compile(source, &options);
        assert_eq!(senseic_compile_result_status(result), SenseicStatus::Ok);
        assert!(senseic_compile_result_bytecode(result).len > 0);
        senseic_compile_result_free(result);
    }

    #[test]
    fn deterministic_bytecode_output() {
        let source = br#"
            fn init:
                entry {
                    stop
                }
            fn main:
                entry {
                    stop
                }
        "#;

        let r1 = compile(source, std::ptr::null());
        let r2 = compile(source, std::ptr::null());

        let b1 = senseic_compile_result_bytecode(r1);
        let b2 = senseic_compile_result_bytecode(r2);
        assert_eq!(b1.len, b2.len);
        let s1 = unsafe { std::slice::from_raw_parts(b1.ptr, b1.len) };
        let s2 = unsafe { std::slice::from_raw_parts(b2.ptr, b2.len) };
        assert_eq!(s1, s2);

        senseic_compile_result_free(r1);
        senseic_compile_result_free(r2);
    }

    #[test]
    fn accessors_readable_multiple_times_before_free() {
        let source = br#"
            fn init:
                entry {
                    stop
                }
            fn main:
                entry {
                    stop
                }
        "#;

        let result = compile(source, std::ptr::null());
        for _ in 0..3 {
            assert_eq!(senseic_compile_result_status(result), SenseicStatus::Ok);
            assert!(senseic_compile_result_bytecode(result).len > 0);
            assert_eq!(senseic_compile_result_message(result).len, 0);
            assert_eq!(senseic_compile_result_diagnostic_count(result), 0);
        }
        senseic_compile_result_free(result);
    }

    #[test]
    fn out_of_bounds_diagnostic_index_returns_zero() {
        let source = br#"
            fn init:
                entry {
                    stop
                }
            data
                bytes
                -0x01
        "#;
        let result = compile(source, std::ptr::null());
        let count = senseic_compile_result_diagnostic_count(result);
        let mut diagnostic = SenseicDiagnostic {
            kind: SenseicDiagnosticKind::Internal,
            span_start: 0,
            span_end: 0,
            message: SenseicByteSlice::empty(),
        };
        assert_eq!(
            senseic_compile_result_diagnostic_get(result, count, &mut diagnostic as *mut _),
            0
        );
        assert_eq!(
            senseic_compile_result_diagnostic_get(result, usize::MAX, &mut diagnostic as *mut _),
            0
        );
        senseic_compile_result_free(result);
    }
}
