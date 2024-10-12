extern crate proc_macro;
use once_cell::sync::Lazy;
use proc_macro::TokenStream;
use quote::quote;
use std::{collections::HashMap, sync::Mutex};
use syn::{parse_macro_input, DeriveInput, FnArg, ItemImpl, Pat, ReturnType};

static FUNC_REGISTRY: Lazy<Mutex<HashMap<String, HashMap<String, Fn>>>> =
    Lazy::new(|| Mutex::new(Default::default()));

fn str_to_ident(s: impl ToString) -> syn::Ident {
    syn::Ident::new(&s.to_string(), proc_macro2::Span::call_site())
}

#[proc_macro_derive(TCPShare)]
pub fn derive_answer_fn(input: TokenStream) -> proc_macro::TokenStream {
    let input: DeriveInput = parse_macro_input!(input as DeriveInput);
    let name: &syn::Ident = &input.ident;
    let name_str = name.to_string();
    let reader_service_name_str = str_to_ident(format!("{}Reader", name_str));
    let writer_service_name_str = str_to_ident(format!("{}Writer", name_str));

    let elements = FUNC_REGISTRY.lock().unwrap();
    let elements = elements.get(&name_str);
    let cases = elements
        .as_ref()
        .map(|v| {
            v.values()
                .map(|v| v.body.parse::<proc_macro2::TokenStream>().unwrap())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let elements = elements
        .map(|v| {
            v.values()
                .map(|v| {
                    let name = str_to_ident(&v.name);
                    let r_type = &v.ret_type;
                    let args = v
                        .args
                        .iter()
                        .filter(|v| v.contains(":"))
                        .map(|arg_str| {
                            let token_stream: proc_macro2::TokenStream = arg_str.parse().unwrap();
                            token_stream
                        })
                        .collect::<Vec<_>>();
                    let data: proc_macro2::TokenStream = v.to_data.parse().unwrap();

                    #[cfg(feature = "async-tcp")]
                    match r_type {
                        Some(r_type) => {
                            let r_type: proc_macro2::TokenStream = r_type.parse().unwrap();
                            quote! {pub async fn #name(&self, #(#args),*) -> Result<#r_type, tcp_struct::Error>{
                                #data
                            }}
                        }
                        None => quote! {pub async fn #name(&self, #(#args),*) -> Result<(), tcp_struct::Error>{
                            #data
                        }},
                    }
                    #[cfg(not(feature = "async-tcp"))]
                    match r_type {
                        Some(r_type) => {
                            let r_type: proc_macro2::TokenStream = r_type.parse().unwrap();
                            quote! {pub fn #name(&self, #(#args),*) -> Result<#r_type, tcp_struct::Error>{
                                #data
                            }}
                        }
                        None => quote! {pub fn #name(&self, #(#args),*) -> Result<(), tcp_struct::Error>{
                            #data
                        }},
                    }

                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let quote = quote! {
        pub struct #reader_service_name_str {
            port: u16,
            head: String,
        }
        pub struct #writer_service_name_str {
            data: std::sync::Arc<tokio::sync::Mutex<#name>>,
        }

        impl tcp_struct::Receiver<#name> for #writer_service_name_str {
            fn request(func: String, data: Vec<u8>, app_data: std::sync::Arc<tokio::sync::Mutex<#name>>) -> std::pin::Pin<Box<dyn std::future::Future<Output = tcp_struct::Result<Vec<u8>>> + Send>> {
                Box::pin(async move {
                match func.as_str() {
                    #(#cases)*
                    _ => Err(tcp_struct::Error::FunctionNotFound)
                }
                })
            }
            fn get_app_data(&self) -> std::sync::Arc<tokio::sync::Mutex<#name>> {
                self.data.clone()
            }
        }

        impl #reader_service_name_str {
            #(#elements)*
        }

        impl #name {
            pub fn read(port: u16, head: &str) -> #reader_service_name_str {
                #reader_service_name_str {
                    port,
                    head: head.to_string()
                }
            }
        }

        impl tcp_struct::Starter for #name {
            async fn start(self, port: u16, header: &str) -> std::io::Result<()> {
                use tcp_struct::Receiver as _;
                #writer_service_name_str {
                    data: std::sync::Arc::new(tokio::sync::Mutex::new(self))
                }.start(port, header).await
            }

            async fn start_from_listener(self, listener: tcp_struct::TcpListener, header: &str) -> std::io::Result<()> {
                use tcp_struct::Receiver as _;
                #writer_service_name_str {
                    data: std::sync::Arc::new(tokio::sync::Mutex::new(self))
                }.start_from_listener(listener, header).await
            }
        }
    };
    quote.into()
}

#[proc_macro_attribute]
pub fn register_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemImpl);

    let struct_name = if let syn::Type::Path(type_path) = &*input.self_ty {
        if let Some(segment) = type_path.path.segments.last() {
            segment.ident.to_string()
        } else {
            "Unknown".to_string()
        }
    } else {
        "Unknown".to_string()
    };

    let func_names: Vec<Fn> = input
        .items
        .iter()
        .filter_map(|item| {
            if let syn::ImplItem::Method(method) = item {
                let asyncronous = method.sig.asyncness.is_some();
                let name = &method.sig.ident;
                let name_str = name.to_string();
                let args: Vec<String> = method
                    .sig
                    .inputs
                    .iter()
                    .map(|arg| quote!(#arg).to_string())
                    .collect();

                let arg_names = method.sig.inputs.iter().flat_map(|v|{
                    match v {
                        syn::FnArg::Receiver(_) => None,
                        syn::FnArg::Typed(pat_type) => {
                            if let Pat::Ident(pat_ident) = &*pat_type.pat {
                                Some(&pat_ident.ident)
                            } else {
                                None
                            }
                        },
                    }
                }).collect::<Vec<_>>();
                let to_data = match arg_names.is_empty() {
                    false => quote! {tcp_struct::encode((#(#arg_names),*))?},
                    true => quote! {vec![]},
                };
                #[cfg(feature = "async-tcp")]
                let to_data = quote! {Ok(tcp_struct::decode(&tcp_struct::send_data(self.port,&self.head,#name_str,#to_data).await?)?)}.to_string();
                #[cfg(not(feature = "async-tcp"))]
                let to_data = quote! {Ok(tcp_struct::decode(&tcp_struct::send_data(self.port,&self.head,#name_str,#to_data)?)?)}.to_string();
                let arg_types: Vec<_> = method.sig.inputs
                    .iter()
                    .filter_map(|arg| {
                        if let FnArg::Typed(pat_type) = arg {
                            let ty = &pat_type.ty;
                            Some(quote! {#ty})
                        } else {
                            None
                        }
                    })
                    .collect();

                let seperator = match arg_names.len() == args.len() {
                    true => {
                        let struct_name = str_to_ident(&struct_name);
                        quote! {#struct_name::#name}},
                    false => quote! {app_data.lock().await.#name},
                };

                let load = match arg_names.is_empty() {
                    true => quote! {},
                    false => quote! {let (#(#arg_names),*) = tcp_struct::decode::<(#(#arg_types),*)>(&data)?;}
                };

                let asyncronous = match asyncronous {
                    true => quote! {.await},
                    false => quote! {}
                };

                let body = quote! {
                    #name_str => {
                        #load
                        tcp_struct::encode(#seperator(#(#arg_names),*)#asyncronous)
                    },
                }.to_string();

                Some(Fn {
                    body,
                    to_data,
                    args,
                    ret_type: match &method.sig.output {
                        ReturnType::Default => None,
                        ReturnType::Type(_, ty) => Some(format!("{}", quote!(#ty))),
                    },
                    name: name_str,
                })
            } else {
                None
            }
        })
        .collect();

    {
        let mut registry = FUNC_REGISTRY.lock().unwrap();
        let map = registry.entry(struct_name).or_default();
        for func_name in func_names {
            map.insert(func_name.name.clone(), func_name);
        }
    }

    TokenStream::from(quote!(#input))
}

struct Fn {
    args: Vec<String>,
    ret_type: Option<String>,
    name: String,
    body: String,
    to_data: String,
}
