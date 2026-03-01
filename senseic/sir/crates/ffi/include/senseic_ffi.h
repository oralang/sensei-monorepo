#ifndef SENSEIC_FFI_H
#define SENSEIC_FFI_H

#pragma once

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

typedef enum SenseicStatus {
    Ok = 0,
    InvalidArgument = 1,
    ParseError = 2,
    EmitError = 3,
    IllegalIrError = 4,
    BackendError = 5,
    Panic = 6,
} SenseicStatus;

typedef enum SenseicDiagnosticKind {
    Parse = 1,
    Emit = 2,
    IllegalIr = 3,
    Backend = 4,
    Internal = 5,
    Panic = 6,
} SenseicDiagnosticKind;

typedef struct SenseicByteSlice {
    const uint8_t *ptr;
    uintptr_t len;
} SenseicByteSlice;

typedef struct SenseicCompileOptions {
    uint8_t init_only;
    struct SenseicByteSlice init_name;
    struct SenseicByteSlice main_name;
    struct SenseicByteSlice optimize_passes;
} SenseicCompileOptions;

typedef struct SenseicCompileResult {
    uint8_t _private[0];
} SenseicCompileResult;

typedef struct SenseicDiagnostic {
    enum SenseicDiagnosticKind kind;
    uint32_t span_start;
    uint32_t span_end;
    struct SenseicByteSlice message;
} SenseicDiagnostic;

uint32_t senseic_abi_version(void);

uint32_t senseic_bytecode_version(void);

struct SenseicByteSlice senseic_compiler_version(void);

struct SenseicCompileOptions senseic_compile_options_default(void);

struct SenseicCompileResult *senseic_compile_sir_to_bytecode(struct SenseicByteSlice source,
                                                             const struct SenseicCompileOptions *options);

void senseic_compile_result_free(struct SenseicCompileResult *result);

enum SenseicStatus senseic_compile_result_status(const struct SenseicCompileResult *result);

struct SenseicByteSlice senseic_compile_result_message(const struct SenseicCompileResult *result);

struct SenseicByteSlice senseic_compile_result_bytecode(const struct SenseicCompileResult *result);

uintptr_t senseic_compile_result_diagnostic_count(const struct SenseicCompileResult *result);

uint8_t senseic_compile_result_diagnostic_get(const struct SenseicCompileResult *result,
                                              uintptr_t index,
                                              struct SenseicDiagnostic *out_diagnostic);

#endif  /* SENSEIC_FFI_H */
