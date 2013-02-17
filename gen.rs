use core::io::WriterUtil;
use std::oldmap::HashMap;

use syntax::ast;
use syntax::codemap::{dummy_sp, dummy_spanned};
use syntax::codemap;
use syntax::ast_util::*;
use syntax::ext::base;
use syntax::ext::build;
use syntax::parse;
use syntax::print::pprust;

use types::*;

struct GenCtx {
    ext_cx: base::ext_ctxt,
    mut unnamed_ty: uint,
    keywords: HashMap<~str, ()>
}

fn rust_id(ctx: &GenCtx, name: ~str) -> (~str, bool) {
    if ctx.keywords.contains_key(&name) {
        (~"_" + name, true)
    } else {
        (name, false)
    }
}

fn unnamed_name(ctx: &GenCtx, name: ~str) -> ~str {
    return if str::is_empty(name) {
        ctx.unnamed_ty += 1;
        fmt!("Unnamed%u", ctx.unnamed_ty)
    } else {
        name
    };
}

fn gen_rs(out: io::Writer, link: &Option<~str>, globs: &[Global]) {
    let ctx = GenCtx { ext_cx: base::mk_ctxt(parse::new_parse_sess(None), ~[]),
                       mut unnamed_ty: 0,
                       keywords: parse::token::keyword_table()
                     };
    ctx.ext_cx.bt_push(codemap::ExpandedFrom({
        call_site: dummy_sp(),
        callie: {name: ~"top", span: None}
    }));

    let mut fs = ~[];
    let mut vs = ~[];
    let mut gs = ~[];
    for globs.each |g| {
        match *g {
            GOther => {}
            GFunc(_) => fs.push(*g),
            GVar(_) => vs.push(*g),
            _ => gs.push(*g)
        }
    }

    let mut defs = ~[];
    gs = remove_redundent_decl(gs);
    for gs.each |g| {
        match *g {
            GType(ti) => defs += ctypedef_to_rs(&ctx, copy ti.name, ti.ty),
            GCompDecl(ci) => {
                ci.name = unnamed_name(&ctx, copy ci.name);
                if ci.cstruct {
                    defs += ctypedef_to_rs(&ctx, ~"Struct_" + ci.name, @TVoid)
                } else {
                    defs += ctypedef_to_rs(&ctx, ~"Union_" + ci.name, @TVoid)
                }
            },
            GComp(ci) => {
                ci.name = unnamed_name(&ctx, copy ci.name);
                if ci.cstruct {
                    defs.push(cstruct_to_rs(&ctx, ~"Struct_" + ci.name,
                                            copy ci.fields))
                } else {
                    defs += cunion_to_rs(&ctx, ~"Union_" + ci.name,
                                         copy ci.fields)
                }
            },
            GEnumDecl(ei) => {
                ei.name = unnamed_name(&ctx, copy ei.name);
                defs += ctypedef_to_rs(&ctx, ~"Enum_" + ei.name, @TVoid)
            },
            GEnum(ei) => {
                ei.name = unnamed_name(&ctx, copy ei.name);
                defs += cenum_to_rs(&ctx, ~"Enum_" + ei.name, copy ei.items,
                                    ei.kind)
            },
            _ => { }
        }
    }

    let vars = do vs.map |v| {
        match *v {
            GVar(vi) => cvar_to_rs(&ctx, copy vi.name, vi.ty),
            _ => { fail!(~"generate global variables") }
        }
    };

    let funcs = do fs.map |f| {
        match *f {
            GFunc(vi) => {
                match *vi.ty {
                    TFunc(rty, ref aty, var) => cfunc_to_rs(&ctx, copy vi.name,
                                                             rty, copy *aty, var),
                    _ => { fail!(~"generate functions") }
                }
            },
            _ => { fail!(~"generate functions") }
        }
    };

    let views = ~[mk_import(&ctx, &[~"core", ~"libc"])];
    defs.push(mk_extern(&ctx, link, vars, funcs));

    let crate = @dummy_spanned(ast::crate_ {
        module: ast::_mod {
            view_items: views,
            items: defs,
        },
        attrs: ~[],
        config: ~[]
    });

    let ps = pprust::rust_printer(out, ctx.ext_cx.parse_sess().interner);
    out.write_line(~"/* automatically generated by rust-bindgen */\n");
    pprust::print_crate_(ps, crate);
}

fn mk_import(ctx: &GenCtx, path: &[~str]) -> @ast::view_item {
    let view = ast::view_item_import(~[
        @dummy_spanned(
            ast::view_path_glob(
                @ast::path {
                   span: dummy_sp(),
                   global: false,
                   idents: path.map(|p| ctx.ext_cx.ident_of(copy *p)),
                   rp: None,
                   types: ~[]
                },
                ctx.ext_cx.next_id()
            )
        )
    ]);

    return @ast::view_item {
              node: view,
              attrs: ~[],
              vis: ast::inherited,
              span: dummy_sp()
           };
}

fn mk_extern(ctx: &GenCtx, link: &Option<~str>,
                           vars: ~[@ast::foreign_item],
                           funcs: ~[@ast::foreign_item]) -> @ast::item {
    let attrs;
    match *link {
        None => attrs = ~[],
        Some(ref l) => {
            let link_args = dummy_spanned(ast::attribute_ {
                style: ast::attr_outer,
                value: dummy_spanned(
                    ast::meta_name_value(
                        ~"link_args",
                        dummy_spanned(ast::lit_str(@(~"-l" + *l)))
                    )
                ),
                is_sugared_doc: false
            });
            attrs = ~[link_args];
        }
    }

    let ext = ast::item_foreign_mod(ast::foreign_mod {
        sort: ast::anonymous,
        abi: ctx.ext_cx.ident_of(~"C"),
        view_items: ~[],
        items: vars + funcs
    });

    return @ast::item {
              ident: ctx.ext_cx.ident_of(~""),
              attrs: attrs,
              id: ctx.ext_cx.next_id(),
              node: ext,
              vis: ast::public,
              span: dummy_sp()
           };
}

fn remove_redundent_decl(gs: &[Global]) -> ~[Global] {
    let typedefs = do gs.filtered |g| {
        match(*g) {
            GType(_) => true,
            _ => false
        }
    };

    return do gs.filtered |g| {
        !do typedefs.any |t| {
            match (*g, *t) {
                (GComp(ci1), GType(ti)) => match *ti.ty {
                    TComp(ci2) => ptr::ref_eq(ci1, ci2) && str::is_empty(ci1.name),
                    _ => false
                },
                (GEnum(ei1), GType(ti)) => match *ti.ty {
                    TEnum(ei2) => ptr::ref_eq(ei1, ei2) && str::is_empty(ei1.name),
                    _ => false
                },
                _ => false
            }
        }
    };
}

fn ctypedef_to_rs(ctx: &GenCtx, name: ~str, ty: @Type) -> ~[@ast::item] {
    fn mk_item(ctx: &GenCtx, name: ~str, ty: @Type) -> @ast::item {
        let rust_name = rust_id(ctx, name).first();
        let rust_ty = cty_to_rs(ctx, ty);
        let base = ast::item_ty(
            @ast::Ty {
                id: ctx.ext_cx.next_id(),
                node: copy rust_ty.node,
                span: dummy_sp(),
            },
            ~[]
        );

        return @ast::item {
                  ident: ctx.ext_cx.ident_of(rust_name),
                  attrs: ~[],
                  id: ctx.ext_cx.next_id(),
                  node: base,
                  vis: ast::public,
                  span: dummy_sp()
               };
    }

    return match *ty {
        TComp(ci) => if str::is_empty(ci.name) {
            ci.name = copy name;
            if ci.cstruct {
                ~[cstruct_to_rs(ctx, name, copy ci.fields)]
            } else {
                cunion_to_rs(ctx, name, copy ci.fields)
            }
        } else {
            ~[mk_item(ctx, name, ty)]
        },
        TEnum(ei) => if str::is_empty(ei.name) {
            ei.name = copy name;
            cenum_to_rs(ctx, name, copy ei.items, ei.kind)
        } else {
            ~[mk_item(ctx, name, ty)]
        },
        _ => ~[mk_item(ctx, name, ty)]
    }
}

fn cstruct_to_rs(ctx: &GenCtx, name: ~str, fields: ~[@FieldInfo]) -> @ast::item {
    let mut unnamed = 0;
    let fs = do fields.map |f| {
        let f_name = if str::is_empty(f.name) {
            unnamed += 1;
            fmt!("unnamed_field%u", unnamed)
        } else {
            rust_id(ctx, copy f.name).first()
        };

        let f_ty = cty_to_rs(ctx, f.ty);

        @dummy_spanned(ast::struct_field_ {
            kind: ast::named_field(
                ctx.ext_cx.ident_of(f_name),
                ast::struct_immutable,
                ast::public
            ),
            id: ctx.ext_cx.next_id(),
            ty: f_ty
        })
    };

    let def = ast::item_struct(
        @ast::struct_def {
           fields: fs,
           dtor: None,
           ctor_id: None
        },
        ~[]
    );

    return @ast::item { ident: ctx.ext_cx.ident_of(rust_id(ctx, name).first()),
              attrs: ~[],
              id: ctx.ext_cx.next_id(),
              node: def,
              vis: ast::public,
              span: dummy_sp()
           };
}

fn cunion_to_rs(ctx: &GenCtx, name: ~str, fields: ~[@FieldInfo]) -> ~[@ast::item] {
    fn mk_item(ctx: &GenCtx, name: ~str, item: ast::item_) -> @ast::item {
        return @ast::item {
                  ident: ctx.ext_cx.ident_of(name),
                  attrs: ~[],
                  id: ctx.ext_cx.next_id(),
                  node: item,
                  vis: ast::public,
                  span: dummy_sp()
               };
    }

    let ext_cx = &ctx.ext_cx;
    let ci = mk_compinfo(copy name, false);
    ci.fields = copy fields;
    let union = @TNamed(mk_typeinfo(copy name, @TComp(ci)));

    let data = @dummy_spanned(ast::struct_field_ {
        kind: ast::named_field(
            ext_cx.ident_of(~"data"),
            ast::struct_immutable,
            ast::public
        ),
        id: ext_cx.next_id(),
        ty: cty_to_rs(ctx, @TArray(@TInt(IUChar), type_size(union)))
    });

    let def = ast::item_struct(
        @ast::struct_def {
           fields: ~[data],
           dtor: None,
           ctor_id: None
        },
        ~[]
    );
    let union_def = mk_item(ctx, rust_id(ctx, name).first(), def);

    let expr = quote_expr!(
        unsafe { cast::reinterpret_cast(&ptr::to_unsafe_ptr(self)) }
    );
    let mut unnamed = 0;
    let fs = do fields.map |f| {
        let f_name = if str::is_empty(f.name) {
            unnamed += 1;
            fmt!("unnamed_field%u", unnamed)
        } else {
            rust_id(ctx, copy f.name).first()
        };

        let ret_ty = cty_to_rs(ctx, @TPtr(f.ty));
        let body = dummy_spanned(ast::blk_ {
            view_items: ~[],
            stmts: ~[],
            expr: Some(expr),
            id: ext_cx.next_id(),
            rules: ast::default_blk
        });

        @ast::method {
            ident: ext_cx.ident_of(f_name),
            attrs: ~[],
            tps: ~[],
            self_ty: dummy_spanned(ast::sty_region(ast::m_imm)),
            purity: ast::impure_fn,
            decl: ast::fn_decl {
                inputs: ~[],
                output: ret_ty,
                cf: ast::return_val
            },
            body: body,
            id: ext_cx.next_id(),
            span: dummy_sp(),
            self_id: union_def.id,
            vis: ast::public
        }
    };

    let methods = ast::item_impl(
        ~[],
        None,
        cty_to_rs(ctx, union),
        fs
    );

    return ~[
        union_def,
        mk_item(ctx, ~"", methods)
    ];
}

fn cenum_to_rs(ctx: &GenCtx, name: ~str, items: ~[@EnumItem], kind: IKind) -> ~[@ast::item] {
    let ty = @TInt(kind);
    let ty_def = ctypedef_to_rs(ctx, rust_id(ctx, name).first(), ty);
    let val_ty = cty_to_rs(ctx, ty);
    let mut def = ty_def;

    for items.each |it| {
        let cst = ast::item_const(
            val_ty,
            build::mk_int(ctx.ext_cx, dummy_sp(), it.val)
        );

        let val_def = @ast::item {
                         ident: ctx.ext_cx.ident_of(rust_id(ctx, copy it.name).first()),
                         attrs: ~[],
                         id: ctx.ext_cx.next_id(),
                         node: cst,
                         vis: ast::public,
                         span: dummy_sp()
                      };

        def.push(val_def);
    }

    return def;
}

fn mk_link_name_attr(name: ~str) -> ast::attribute {
    let lit = dummy_spanned(ast::lit_str(@(name)));
    let attr_val = dummy_spanned(ast::meta_name_value(~"link_name", lit));
    let attr = ast::attribute_ {
        style: ast::attr_outer,
        value: attr_val,
        is_sugared_doc: false
    };
    dummy_spanned(attr)
}

fn cvar_to_rs(ctx: &GenCtx, name: ~str, ty: @Type) -> @ast::foreign_item {
    let (rust_name, was_mangled) = rust_id(ctx, copy name);

    let mut attrs = ~[];
    if was_mangled {
        attrs.push(mk_link_name_attr(name));
    }

    return @ast::foreign_item {
              ident: ctx.ext_cx.ident_of(rust_name),
              attrs: attrs,
              node: ast::foreign_item_const(cty_to_rs(ctx, ty)),
              id: ctx.ext_cx.next_id(),
              span: dummy_sp(),
              vis: ast::public,
           };
}

fn cfunc_to_rs(ctx: &GenCtx, name: ~str, rty: @Type,
                                         aty: ~[(~str, @Type)],
                                         _var: bool) -> @ast::foreign_item {
    let ret = match *rty {
        TVoid => @ast::Ty {
            id: ctx.ext_cx.next_id(),
            node: ast::ty_nil,
            span: dummy_sp()
        },
        _ => cty_to_rs(ctx, rty)
    };

    let mut unnamed = 0;
    let args = do aty.map |arg| {
        let (n, t) = copy *arg;

        let arg_name = if str::is_empty(n) {
            unnamed += 1;
            fmt!("arg%u", unnamed)
        } else {
            rust_id(ctx, n).first()
        };

        let arg_ty = cty_to_rs(ctx, t);

        ast::arg {
            mode: ast::expl(ast::by_val),
            is_mutbl: false,
            ty: arg_ty,
            pat: @ast::pat {
                 id: ctx.ext_cx.next_id(),
                 node: ast::pat_ident(
                     ast::bind_by_copy,
                     @ast::path {
                         span: dummy_sp(),
                         global: false,
                         idents: ~[ctx.ext_cx.ident_of(arg_name)],
                         rp: None,
                         types: ~[]
                     },
                     None
                 ),
                 span: dummy_sp()
            },
            id: ctx.ext_cx.next_id()
        }
    };

    let decl = ast::foreign_item_fn(
        ast::fn_decl {
            inputs: args,
            output: ret,
            cf: ast::return_val
        },
        ast::impure_fn,
        ~[]
    );

    let (rust_name, was_mangled) = rust_id(ctx, copy name);

    let mut attrs = ~[];
    if was_mangled {
        attrs.push(mk_link_name_attr(name));
    }

    return @ast::foreign_item {
              ident: ctx.ext_cx.ident_of(rust_name),
              attrs: attrs,
              node: decl,
              id: ctx.ext_cx.next_id(),
              span: dummy_sp(),
              vis: ast::public,
           };
}

fn cty_to_rs(ctx: &GenCtx, ty: @Type) -> @ast::Ty {
    return match *ty {
        TVoid => mk_ty(ctx, ~"c_void"),
        TInt(i) => match i {
            IBool => mk_ty(ctx, ~"c_int"),
            ISChar => mk_ty(ctx, ~"c_schar"),
            IUChar => mk_ty(ctx, ~"c_uchar"),
            IInt => mk_ty(ctx, ~"c_int"),
            IUInt => mk_ty(ctx, ~"c_uint"),
            IShort => mk_ty(ctx, ~"c_short"),
            IUShort => mk_ty(ctx, ~"c_ushort"),
            ILong => mk_ty(ctx, ~"c_long"),
            IULong => mk_ty(ctx, ~"c_ulong"),
            ILongLong => mk_ty(ctx, ~"c_longlong"),
            IULongLong => mk_ty(ctx, ~"c_ulonglong")
        },
        TFloat(f) => match f {
            FFloat => mk_ty(ctx, ~"c_float"),
            FDouble => mk_ty(ctx, ~"c_double")
        },
        TPtr(t) => mk_ptrty(ctx, cty_to_rs(ctx, t)),
        TArray(t, s) => mk_arrty(ctx, cty_to_rs(ctx, t), s),
        TFunc(_, _, _) => mk_fnty(ctx),
        TNamed(ti) => mk_ty(ctx, rust_id(ctx, copy ti.name).first()),
        TComp(ci) => {
            ci.name = unnamed_name(ctx, copy ci.name);
            if ci.cstruct {
                mk_ty(ctx, ~"Struct_" + ci.name)
            } else {
                mk_ty(ctx, ~"Union_" + ci.name)
            }
        },
        TEnum(ei) => {
            ei.name = unnamed_name(ctx, copy ei.name);
            mk_ty(ctx, ~"Enum_" + ei.name)
        }
    };
}

fn mk_ty(ctx: &GenCtx, name: ~str) -> @ast::Ty {
    let ty = ast::ty_path(
        @ast::path {
            span: dummy_sp(),
            global: false,
            idents: ~[ctx.ext_cx.ident_of(name)],
            rp: None,
            types: ~[]
        },
        ctx.ext_cx.next_id()
    );

    return @ast::Ty {
        id: ctx.ext_cx.next_id(),
        node: ty,
        span: dummy_sp()
    };
}

fn mk_ptrty(ctx: &GenCtx, base: @ast::Ty) -> @ast::Ty {
    let ty = ast::ty_ptr(ast::mt{
        ty: base,
        mutbl: ast::m_imm
    });

    return @ast::Ty {
        id: ctx.ext_cx.next_id(),
        node: ty,
        span: dummy_sp()
    };
}

fn mk_arrty(ctx: &GenCtx, base: @ast::Ty, n: uint) -> @ast::Ty {
    let ty = ast::ty_fixed_length_vec(
        ast::mt {
            ty: base,
            mutbl: ast::m_imm
        },
        n
    );

    return @ast::Ty {
        id: ctx.ext_cx.next_id(),
        node: ty,
        span: dummy_sp()
    };
}

fn mk_fnty(ctx: &GenCtx) -> @ast::Ty {
    let @ast::Ty{node: node, _} = mk_ptrty(ctx, mk_ty(ctx, ~"u8"));

    return @ast::Ty {
        id: ctx.ext_cx.next_id(),
        node: node,
        span: dummy_sp()
    };
}
