(globalThis.TURBOPACK = globalThis.TURBOPACK || []).push(["output/turbopack_crates_turbopack-tests_tests_snapshot_imports_dynamic_input_041623._.js", {

"[project]/turbopack/crates/turbopack-tests/tests/snapshot/imports/dynamic/input/vercel.cjs [test] (ecmascript)": (function(__turbopack_context__) {

var { r: __turbopack_require__, f: __turbopack_module_context__, i: __turbopack_import__, s: __turbopack_esm__, v: __turbopack_export_value__, n: __turbopack_export_namespace__, c: __turbopack_cache__, M: __turbopack_modules__, l: __turbopack_load__, j: __turbopack_dynamic__, P: __turbopack_resolve_absolute_path__, U: __turbopack_relative_url__, R: __turbopack_resolve_module_id_path__, b: __turbopack_worker_blob_url__, g: global, __dirname, m: module, e: exports, t: require } = __turbopack_context__;
module.exports = "turbopack";
}),
"[project]/turbopack/crates/turbopack-tests/tests/snapshot/imports/dynamic/input/index.js [test] (ecmascript)": (function(__turbopack_context__) {

var { r: __turbopack_require__, f: __turbopack_module_context__, i: __turbopack_import__, s: __turbopack_esm__, v: __turbopack_export_value__, n: __turbopack_export_namespace__, c: __turbopack_cache__, M: __turbopack_modules__, l: __turbopack_load__, j: __turbopack_dynamic__, P: __turbopack_resolve_absolute_path__, U: __turbopack_relative_url__, R: __turbopack_resolve_module_id_path__, b: __turbopack_worker_blob_url__, g: global, __dirname, m: module, e: exports, t: require } = __turbopack_context__;
__turbopack_require__("[project]/turbopack/crates/turbopack-tests/tests/snapshot/imports/dynamic/input/vercel.mjs [test] (ecmascript, async loader)")(__turbopack_import__).then(console.log);
__turbopack_require__("[project]/turbopack/crates/turbopack-tests/tests/snapshot/imports/dynamic/input/vercel.mjs [test] (ecmascript, async loader)")(__turbopack_import__).then(console.log);
console.log(__turbopack_require__("[project]/turbopack/crates/turbopack-tests/tests/snapshot/imports/dynamic/input/vercel.cjs [test] (ecmascript)"));
// turbopack shouldn't attempt to bundle these, and they should be preserved as dynamic esm imports
// in the output
import(/* webpackIgnore: true */ "./ignore.mjs");
import(/* turbopackIgnore: true */ "./ignore.mjs");
// this should work for cjs requires too
__turbopack_require__("[project]/turbopack/crates/turbopack-tests/tests/snapshot/imports/dynamic/input/vercel.mjs [test] (ecmascript, async loader)")(__turbopack_import__).then(console.log);
require(/* webpackIgnore: true */ "./ignore.cjs");
require(/* turbopackIgnore: true */ "./ignore.cjs");
}),
}]);

//# sourceMappingURL=turbopack_crates_turbopack-tests_tests_snapshot_imports_dynamic_input_041623._.js.map