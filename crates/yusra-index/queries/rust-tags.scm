; Query tag Rust untuk repo map. Definisi sama dengan `tags.scm` bawaan
; tree-sitter-rust 0.24; rujukan ditambah panggilan ber-path dan rujukan tipe.
;
; Semua posisi definisi `type_identifier` di grammar memakai field `name`
; (struct, enum, union, trait, type alias, associated type, parameter
; generic), jadi pola rujukan tipe di bawah sengaja tidak memakai field `name`
; kecuali pada `scoped_type_identifier` dan `struct_expression`.

; definisi ADT

(struct_item
    name: (type_identifier) @name) @definition.class

(enum_item
    name: (type_identifier) @name) @definition.class

(union_item
    name: (type_identifier) @name) @definition.class

(type_item
    name: (type_identifier) @name) @definition.class

; definisi method dan fungsi

(declaration_list
    (function_item
        name: (identifier) @name) @definition.method)

(function_item
    name: (identifier) @name) @definition.function

(trait_item
    name: (type_identifier) @name) @definition.interface

(mod_item
    name: (identifier) @name) @definition.module

(macro_definition
    name: (identifier) @name) @definition.macro

; panggilan: `f()`, `x.f()`, `a::f()`, `f::<T>()`, `x.f::<T>()`, `a::f::<T>()`

(call_expression
    function: (identifier) @name) @reference.call

(call_expression
    function: (field_expression
        field: (field_identifier) @name)) @reference.call

(call_expression
    function: (scoped_identifier
        name: (identifier) @name)) @reference.call

(call_expression
    function: (generic_function
        function: (identifier) @name)) @reference.call

(call_expression
    function: (generic_function
        function: (field_expression
            field: (field_identifier) @name))) @reference.call

(call_expression
    function: (generic_function
        function: (scoped_identifier
            name: (identifier) @name))) @reference.call

(macro_invocation
    macro: (identifier) @name) @reference.call

(macro_invocation
    macro: (scoped_identifier
        name: (identifier) @name)) @reference.call

; segmen terakhir path berhuruf kapital adalah tipe: `Session` pada
; `Session::new()`, `State` pada `state::State::Active`

((scoped_identifier
    path: (identifier) @name) @reference.type
    (#match? @name "^[A-Z]")
    (#not-eq? @name "Self"))

((scoped_identifier
    path: (scoped_identifier
        name: (identifier) @name)) @reference.type
    (#match? @name "^[A-Z]")
    (#not-eq? @name "Self"))

; rujukan tipe: parameter, `let`, field, const/static, nilai type alias,
; tipe dan trait `impl`, `&T`, `*T`, `Foo<..>`, cast, pola struct

((_
    type: (type_identifier) @name) @reference.type
    (#not-eq? @name "Self"))

(_
    trait: (type_identifier) @name) @reference.type

((_
    return_type: (type_identifier) @name) @reference.type
    (#not-eq? @name "Self"))

(_
    element: (type_identifier) @name) @reference.type

(_
    alias: (type_identifier) @name) @reference.type

(_
    default_type: (type_identifier) @name) @reference.type

(scoped_type_identifier
    name: (type_identifier) @name) @reference.type

((struct_expression
    name: (type_identifier) @name) @reference.type
    (#not-eq? @name "Self"))

; tipe tanpa field: argumen generic, tuple, bound trait, `impl A + B`,
; parameter `fn(A) -> B`, `<T as Trait>`

(type_arguments
    (type_identifier) @name) @reference.type

(tuple_type
    (type_identifier) @name) @reference.type

(trait_bounds
    (type_identifier) @name) @reference.type

(bounded_type
    (type_identifier) @name) @reference.type

(parameters
    (type_identifier) @name) @reference.type

(bracketed_type
    (type_identifier) @name) @reference.type
