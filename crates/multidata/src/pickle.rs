//! Lecteur de pickle Python, sûr : il **n'exécute jamais de code**.
//!
//! Un pickle est un petit programme pour une machine à pile : chaque opcode
//! empile une valeur, ou combine les valeurs du haut de la pile. Python peut y
//! appeler n'importe quelle fonction (c'est ce qui rend un pickle piégé
//! dangereux). Ici, on ne fait que **construire des données** :
//! - les seules classes acceptées sont celles de la liste blanche du serveur
//!   officiel (`RestrictedUnpickler` dans `Utils.py`) ;
//! - un objet de ces classes n'est pas « appelé » : on garde juste son nom de
//!   classe et ses arguments, et le modèle de partie les interprète ensuite ;
//! - le nombre d'éléments créés est limité (voir [`Limits`]).
//!
//! # Arène
//!
//! Les valeurs sont rangées dans une liste (une « arène ») et désignées par leur
//! numéro, [`NodeId`]. Une liste ou un dictionnaire contient des numéros, pas les
//! valeurs elles-mêmes. Deux raisons :
//! - le pickle peut faire référence plusieurs fois à la même valeur (le « mémo »),
//!   et la modifier après coup : avec des numéros, tout le monde voit la même ;
//! - pas de copies : un pickle piégé ne peut pas faire exploser la mémoire en
//!   répétant mille fois la même grosse liste.

use std::collections::BTreeMap;

use crate::error::{PickleError, PickleErrorKind};

/// Numéro d'une valeur dans l'arène d'un [`PickleDocument`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(u32);

impl NodeId {
    /// Position dans l'arène.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Classes Python autorisées (liste blanche du serveur officiel, Archipelago 0.6.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyClass {
    NetworkItem,
    NetworkSlot,
    SlotType,
    ClientStatus,
    Hint,
    HintStatus,
    /// `collections.Counter` (un dictionnaire de compteurs).
    Counter,
}

impl PyClass {
    /// Nom complet de la classe, comme en Python : `module.Nom`.
    pub fn path(self) -> &'static str {
        match self {
            PyClass::NetworkItem => "NetUtils.NetworkItem",
            PyClass::NetworkSlot => "NetUtils.NetworkSlot",
            PyClass::SlotType => "NetUtils.SlotType",
            PyClass::ClientStatus => "NetUtils.ClientStatus",
            PyClass::Hint => "NetUtils.Hint",
            PyClass::HintStatus => "NetUtils.HintStatus",
            PyClass::Counter => "collections.Counter",
        }
    }
}

/// Une référence à une classe ou une fonction autorisée (opcodes `GLOBAL`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Global {
    Class(PyClass),
    /// `builtins.set` : seulement pour fabriquer un ensemble (protocoles anciens).
    Set,
    /// `builtins.frozenset`.
    FrozenSet,
}

impl Global {
    /// Vérifie un nom de classe contre la liste blanche.
    fn lookup(module: &str, name: &str) -> Option<Global> {
        let found = match (module, name) {
            ("builtins", "set") => Global::Set,
            ("builtins", "frozenset") => Global::FrozenSet,
            ("collections", "Counter") => Global::Class(PyClass::Counter),
            ("NetUtils", "NetworkItem") => Global::Class(PyClass::NetworkItem),
            ("NetUtils", "NetworkSlot") => Global::Class(PyClass::NetworkSlot),
            ("NetUtils", "SlotType") => Global::Class(PyClass::SlotType),
            ("NetUtils", "ClientStatus") => Global::Class(PyClass::ClientStatus),
            ("NetUtils", "Hint") => Global::Class(PyClass::Hint),
            ("NetUtils", "HintStatus") => Global::Class(PyClass::HintStatus),
            // Le serveur officiel accepte aussi les classes d'options (modules
            // `*options`), mais seulement pour le site web : jamais dans une seed.
            _ => return None,
        };
        Some(found)
    }
}

/// Une valeur Python lue dans le pickle.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    None,
    Bool(bool),
    Int(i64),
    /// Entier qui ne tient pas dans un `i64` : octets en petit-boutiste,
    /// complément à deux, sans octet inutile (comme Python l'écrit).
    BigInt(Vec<u8>),
    Float(f64),
    Str(String),
    Bytes(Vec<u8>),
    List(Vec<NodeId>),
    Tuple(Vec<NodeId>),
    /// Paires clé → valeur, dans l'ordre du pickle.
    Dict(Vec<(NodeId, NodeId)>),
    Set(Vec<NodeId>),
    FrozenSet(Vec<NodeId>),
    /// Classe ou fonction référencée, pas encore utilisée.
    Global(Global),
    /// Objet d'une classe autorisée : on garde ses arguments de construction
    /// et son état (`BUILD`), sans rien exécuter.
    Object {
        class: PyClass,
        args: Vec<NodeId>,
        state: Option<NodeId>,
    },
}

/// Limites de sécurité du lecteur.
#[derive(Debug, Clone)]
pub struct Limits {
    /// Nombre maximal de valeurs créées (protège la mémoire).
    pub max_nodes: usize,
    /// Taille maximale d'un entier, en octets.
    pub max_int_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            // Une seed solo en crée environ 3 000 ; 4 millions laissent une très
            // grande marge, pour quelques centaines de Mio de mémoire au pire.
            max_nodes: 4_000_000,
            max_int_bytes: 64,
        }
    }
}

/// Résultat de la lecture : l'arène et la valeur racine.
#[derive(Debug, Clone)]
pub struct PickleDocument {
    nodes: Vec<Node>,
    root: NodeId,
}

impl PickleDocument {
    /// La valeur de premier niveau (pour une seed : un dictionnaire).
    pub fn root(&self) -> NodeId {
        self.root
    }

    /// La valeur qui porte ce numéro.
    ///
    /// Un `NodeId` vient toujours de ce document, donc l'indice est valide.
    /// (S'il venait d'un autre document, on renverrait `Node::None` plutôt que de planter.)
    pub fn node(&self, id: NodeId) -> &Node {
        self.nodes.get(id.index()).unwrap_or(&Node::None)
    }

    /// Nombre de valeurs dans l'arène.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Vrai si l'arène est vide (jamais le cas pour un document lu avec succès).
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// Lit un pickle complet.
pub fn parse(data: &[u8], limits: &Limits) -> Result<PickleDocument, PickleError> {
    let mut parser = Parser {
        data,
        pos: 0,
        nodes: Vec::new(),
        stack: Vec::new(),
        marks: Vec::new(),
        memo: BTreeMap::new(),
        limits,
    };
    loop {
        // Position de l'opcode en cours, pour les messages d'erreur.
        let at = parser.pos;
        match parser.step() {
            Ok(Step::Continue) => {}
            Ok(Step::Stop) => return parser.finish().map_err(|kind| PickleError { at, kind }),
            Err(kind) => return Err(PickleError { at, kind }),
        }
    }
}

/// Ce que fait la boucle après un opcode.
enum Step {
    Continue,
    Stop,
}

/// État de la machine à pile pendant la lecture.
///
/// `'a` : le lecteur emprunte les données et les limites le temps de la lecture.
struct Parser<'a> {
    data: &'a [u8],
    pos: usize,
    nodes: Vec<Node>,
    stack: Vec<NodeId>,
    /// Hauteurs de pile enregistrées par `MARK`.
    marks: Vec<usize>,
    memo: BTreeMap<u32, NodeId>,
    limits: &'a Limits,
}

type Res<T> = Result<T, PickleErrorKind>;

impl Parser<'_> {
    // --- Lecture des octets -------------------------------------------------

    fn bytes(&mut self, n: usize) -> Res<&[u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or(PickleErrorKind::UnexpectedEof)?;
        let slice = self
            .data
            .get(self.pos..end)
            .ok_or(PickleErrorKind::UnexpectedEof)?;
        self.pos = end;
        Ok(slice)
    }

    /// Lit `N` octets dans un tableau de taille fixe.
    fn array<const N: usize>(&mut self) -> Res<[u8; N]> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.bytes(N)?);
        Ok(out)
    }

    fn u8(&mut self) -> Res<u8> {
        Ok(self.array::<1>()?[0])
    }

    fn u16(&mut self) -> Res<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Res<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Res<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    /// Longueur sur 8 octets, convertie en `usize` sans débordement.
    fn len64(&mut self) -> Res<usize> {
        usize::try_from(self.u64()?).map_err(|_| PickleErrorKind::UnexpectedEof)
    }

    fn string(&mut self, n: usize) -> Res<String> {
        let raw = self.bytes(n)?.to_vec();
        String::from_utf8(raw).map_err(|_| PickleErrorKind::InvalidUtf8)
    }

    /// Ligne de texte terminée par `\n` (opcode `GLOBAL`, protocoles anciens).
    fn line(&mut self) -> Res<String> {
        let rest = self
            .data
            .get(self.pos..)
            .ok_or(PickleErrorKind::UnexpectedEof)?;
        let len = rest
            .iter()
            .position(|b| *b == b'\n')
            .ok_or(PickleErrorKind::UnexpectedEof)?;
        let text = self.string(len)?;
        self.pos += 1; // le `\n`
        Ok(text)
    }

    // --- Arène et pile ------------------------------------------------------

    /// Range une nouvelle valeur dans l'arène.
    fn add(&mut self, node: Node) -> Res<NodeId> {
        if self.nodes.len() >= self.limits.max_nodes {
            return Err(PickleErrorKind::TooManyNodes(self.limits.max_nodes));
        }
        // La limite tient dans un u32 en pratique ; on vérifie quand même.
        let id = u32::try_from(self.nodes.len())
            .map_err(|_| PickleErrorKind::TooManyNodes(self.limits.max_nodes))?;
        self.nodes.push(node);
        Ok(NodeId(id))
    }

    fn push(&mut self, node: Node) -> Res<()> {
        let id = self.add(node)?;
        self.stack.push(id);
        Ok(())
    }

    fn pop(&mut self) -> Res<NodeId> {
        // Une marque protège la partie de la pile qui est en dessous.
        if self.marks.last() == Some(&self.stack.len()) {
            return Err(PickleErrorKind::StackUnderflow);
        }
        self.stack.pop().ok_or(PickleErrorKind::StackUnderflow)
    }

    fn top(&self) -> Res<NodeId> {
        self.stack
            .last()
            .copied()
            .ok_or(PickleErrorKind::StackUnderflow)
    }

    /// Retire tout ce qui est au-dessus de la dernière marque.
    fn pop_mark(&mut self) -> Res<Vec<NodeId>> {
        let height = self.marks.pop().ok_or(PickleErrorKind::MissingMark)?;
        if height > self.stack.len() {
            return Err(PickleErrorKind::StackUnderflow);
        }
        Ok(self.stack.split_off(height))
    }

    fn node_mut(&mut self, id: NodeId) -> Res<&mut Node> {
        self.nodes
            .get_mut(id.index())
            .ok_or(PickleErrorKind::StackUnderflow)
    }

    fn node(&self, id: NodeId) -> Res<&Node> {
        self.nodes
            .get(id.index())
            .ok_or(PickleErrorKind::StackUnderflow)
    }

    fn memo_put(&mut self, key: u32) -> Res<()> {
        let id = self.top()?;
        self.memo.insert(key, id);
        Ok(())
    }

    fn memo_get(&mut self, key: u32) -> Res<()> {
        let id = *self.memo.get(&key).ok_or(PickleErrorKind::BadMemo(key))?;
        // On empile le même numéro : pas de copie.
        self.stack.push(id);
        Ok(())
    }

    // --- Conteneurs ---------------------------------------------------------

    fn append(&mut self, items: Vec<NodeId>) -> Res<()> {
        let target = self.top()?;
        match self.node_mut(target)? {
            Node::List(list) => {
                list.extend(items);
                Ok(())
            }
            _ => Err(PickleErrorKind::WrongType("liste")),
        }
    }

    fn set_items(&mut self, items: Vec<NodeId>) -> Res<()> {
        if items.len() % 2 != 0 {
            return Err(PickleErrorKind::StackUnderflow);
        }
        // `chunks(2)` : les éléments vont par paires (clé, valeur).
        let pairs: Vec<(NodeId, NodeId)> = items.chunks(2).map(|p| (p[0], p[1])).collect();
        let target = self.top()?;
        match self.node_mut(target)? {
            Node::Dict(dict) => {
                dict.extend(pairs);
                Ok(())
            }
            _ => Err(PickleErrorKind::WrongType("dictionnaire")),
        }
    }

    fn add_items(&mut self, items: Vec<NodeId>) -> Res<()> {
        let target = self.top()?;
        match self.node_mut(target)? {
            Node::Set(set) => {
                set.extend(items);
                Ok(())
            }
            _ => Err(PickleErrorKind::WrongType("ensemble")),
        }
    }

    /// Contenu d'un tuple déjà dans l'arène.
    fn tuple_items(&self, id: NodeId) -> Res<Vec<NodeId>> {
        match self.node(id)? {
            Node::Tuple(items) => Ok(items.clone()),
            _ => Err(PickleErrorKind::WrongType("tuple")),
        }
    }

    fn global(&self, id: NodeId) -> Res<Global> {
        match self.node(id)? {
            Node::Global(global) => Ok(*global),
            _ => Err(PickleErrorKind::WrongType("classe")),
        }
    }

    fn text(&self, id: NodeId) -> Res<String> {
        match self.node(id)? {
            Node::Str(text) => Ok(text.clone()),
            _ => Err(PickleErrorKind::WrongType("texte")),
        }
    }

    fn push_global(&mut self, module: String, name: String) -> Res<()> {
        match Global::lookup(&module, &name) {
            Some(global) => self.push(Node::Global(global)),
            None => Err(PickleErrorKind::ForbiddenGlobal { module, name }),
        }
    }

    /// `REDUCE` : « appelle » une classe autorisée avec un tuple d'arguments.
    fn reduce(&mut self) -> Res<()> {
        let args = self.pop()?;
        let callable = self.pop()?;
        let args = self.tuple_items(args)?;
        let node = match self.global(callable)? {
            // `set(liste)` ou `frozenset(liste)` : on construit l'ensemble.
            Global::Set | Global::FrozenSet => {
                let items = match args.as_slice() {
                    [] => Vec::new(),
                    [iterable] => match self.node(*iterable)? {
                        Node::List(items) | Node::Tuple(items) => items.clone(),
                        _ => return Err(PickleErrorKind::WrongType("liste")),
                    },
                    _ => return Err(PickleErrorKind::WrongType("un seul argument")),
                };
                if self.global(callable)? == Global::Set {
                    Node::Set(items)
                } else {
                    Node::FrozenSet(items)
                }
            }
            Global::Class(class) => Node::Object {
                class,
                args,
                state: None,
            },
        };
        self.push(node)
    }

    /// `NEWOBJ` : `cls.__new__(cls, *args)`, utilisé pour les `NamedTuple`.
    fn new_obj(&mut self) -> Res<()> {
        let args = self.pop()?;
        let class = self.pop()?;
        let args = self.tuple_items(args)?;
        match self.global(class)? {
            Global::Class(class) => self.push(Node::Object {
                class,
                args,
                state: None,
            }),
            _ => Err(PickleErrorKind::WrongType("classe")),
        }
    }

    /// `BUILD` : donne son état à l'objet du haut de la pile.
    fn build(&mut self) -> Res<()> {
        let state = self.pop()?;
        let target = self.top()?;
        match self.node_mut(target)? {
            Node::Object { state: slot, .. } => {
                *slot = Some(state);
                Ok(())
            }
            _ => Err(PickleErrorKind::WrongType("objet")),
        }
    }

    /// Entier en complément à deux, petit-boutiste (`LONG1`, `LONG4`).
    fn long(&mut self, len: usize) -> Res<()> {
        if len > self.limits.max_int_bytes {
            return Err(PickleErrorKind::IntegerTooLarge(len));
        }
        let mut bytes = self.bytes(len)?.to_vec();
        // On retire les octets de signe inutiles (forme la plus courte).
        while bytes.len() > 1 {
            let last = bytes[bytes.len() - 1];
            let prev_negative = bytes[bytes.len() - 2] & 0x80 != 0;
            if (last == 0x00 && !prev_negative) || (last == 0xff && prev_negative) {
                bytes.pop();
            } else {
                break;
            }
        }
        if bytes.len() > 8 {
            return self.push(Node::BigInt(bytes));
        }
        // Extension du signe sur 8 octets, puis lecture d'un i64.
        let negative = bytes.last().is_some_and(|b| b & 0x80 != 0);
        let mut full = [if negative { 0xff } else { 0x00 }; 8];
        full[..bytes.len()].copy_from_slice(&bytes);
        self.push(Node::Int(i64::from_le_bytes(full)))
    }

    // --- Boucle principale --------------------------------------------------

    /// Exécute un opcode.
    fn step(&mut self) -> Res<Step> {
        let opcode = self.u8()?;
        match opcode {
            // Structure
            0x80 => {
                // PROTO
                let version = self.u8()?;
                if !(2..=5).contains(&version) {
                    return Err(PickleErrorKind::UnsupportedProtocol(version));
                }
            }
            0x95 => {
                // FRAME : taille du bloc suivant, simple indication.
                self.u64()?;
            }
            b'.' => return Ok(Step::Stop),
            b'(' => self.marks.push(self.stack.len()),
            b'0' => {
                // POP
                self.pop()?;
            }
            b'1' => {
                // POP_MARK
                self.pop_mark()?;
            }

            // Valeurs simples
            b'N' => self.push(Node::None)?,
            0x88 => self.push(Node::Bool(true))?,
            0x89 => self.push(Node::Bool(false))?,
            b'J' => {
                let value = i32::from_le_bytes(self.array()?);
                self.push(Node::Int(i64::from(value)))?;
            }
            b'K' => {
                let value = self.u8()?;
                self.push(Node::Int(i64::from(value)))?;
            }
            b'M' => {
                let value = self.u16()?;
                self.push(Node::Int(i64::from(value)))?;
            }
            0x8a => {
                // LONG1
                let len = usize::from(self.u8()?);
                self.long(len)?;
            }
            0x8b => {
                // LONG4 : longueur signée ; négative = invalide.
                let len = i32::from_le_bytes(self.array()?);
                let len = usize::try_from(len).map_err(|_| PickleErrorKind::UnexpectedEof)?;
                self.long(len)?;
            }
            b'G' => {
                // BINFLOAT : 8 octets, gros-boutiste.
                let value = f64::from_be_bytes(self.array()?);
                self.push(Node::Float(value))?;
            }
            0x8c => {
                // SHORT_BINUNICODE
                let len = usize::from(self.u8()?);
                let text = self.string(len)?;
                self.push(Node::Str(text))?;
            }
            b'X' => {
                // BINUNICODE
                let len = self.u32()? as usize;
                let text = self.string(len)?;
                self.push(Node::Str(text))?;
            }
            0x8d => {
                // BINUNICODE8
                let len = self.len64()?;
                let text = self.string(len)?;
                self.push(Node::Str(text))?;
            }
            b'C' => {
                // SHORT_BINBYTES
                let len = usize::from(self.u8()?);
                let bytes = self.bytes(len)?.to_vec();
                self.push(Node::Bytes(bytes))?;
            }
            b'B' => {
                // BINBYTES
                let len = self.u32()? as usize;
                let bytes = self.bytes(len)?.to_vec();
                self.push(Node::Bytes(bytes))?;
            }
            0x8e => {
                // BINBYTES8
                let len = self.len64()?;
                let bytes = self.bytes(len)?.to_vec();
                self.push(Node::Bytes(bytes))?;
            }

            // Listes
            b']' => self.push(Node::List(Vec::new()))?,
            b'a' => {
                let item = self.pop()?;
                self.append(vec![item])?;
            }
            b'e' => {
                let items = self.pop_mark()?;
                self.append(items)?;
            }
            b'l' => {
                let items = self.pop_mark()?;
                self.push(Node::List(items))?;
            }

            // Tuples
            b')' => self.push(Node::Tuple(Vec::new()))?,
            b't' => {
                let items = self.pop_mark()?;
                self.push(Node::Tuple(items))?;
            }
            0x85..=0x87 => {
                // TUPLE1, TUPLE2, TUPLE3
                let count = usize::from(opcode - 0x84);
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.pop()?);
                }
                items.reverse();
                self.push(Node::Tuple(items))?;
            }

            // Dictionnaires
            b'}' => self.push(Node::Dict(Vec::new()))?,
            b's' => {
                let value = self.pop()?;
                let key = self.pop()?;
                self.set_items(vec![key, value])?;
            }
            b'u' => {
                let items = self.pop_mark()?;
                self.set_items(items)?;
            }
            b'd' => {
                let items = self.pop_mark()?;
                self.push(Node::Dict(Vec::new()))?;
                self.set_items(items)?;
            }

            // Ensembles
            0x8f => self.push(Node::Set(Vec::new()))?,
            0x90 => {
                let items = self.pop_mark()?;
                self.add_items(items)?;
            }
            0x91 => {
                let items = self.pop_mark()?;
                self.push(Node::FrozenSet(items))?;
            }

            // Mémo
            0x94 => {
                // MEMOIZE : la clé est le nombre d'entrées déjà mémorisées.
                let key = u32::try_from(self.memo.len())
                    .map_err(|_| PickleErrorKind::TooManyNodes(self.limits.max_nodes))?;
                self.memo_put(key)?;
            }
            b'q' => {
                let key = u32::from(self.u8()?);
                self.memo_put(key)?;
            }
            b'r' => {
                let key = self.u32()?;
                self.memo_put(key)?;
            }
            b'h' => {
                let key = u32::from(self.u8()?);
                self.memo_get(key)?;
            }
            b'j' => {
                let key = self.u32()?;
                self.memo_get(key)?;
            }

            // Classes et objets
            0x93 => {
                // STACK_GLOBAL : nom du module et de la classe sur la pile.
                let name = self.pop()?;
                let module = self.pop()?;
                let (module, name) = (self.text(module)?, self.text(name)?);
                self.push_global(module, name)?;
            }
            b'c' => {
                // GLOBAL : nom du module et de la classe sur deux lignes.
                let module = self.line()?;
                let name = self.line()?;
                self.push_global(module, name)?;
            }
            b'R' => self.reduce()?,
            0x81 => self.new_obj()?,
            b'b' => self.build()?,

            // Tout le reste est refusé, en particulier les opcodes qui
            // appellent du code (INST, OBJ, NEWOBJ_EX, PERSID…).
            other => return Err(PickleErrorKind::UnknownOpcode(other)),
        }
        Ok(Step::Continue)
    }

    /// Vérifie l'état final après `STOP`.
    fn finish(self) -> Res<PickleDocument> {
        if self.pos != self.data.len() {
            return Err(PickleErrorKind::TrailingData);
        }
        match self.stack.as_slice() {
            [root] if self.marks.is_empty() => Ok(PickleDocument {
                root: *root,
                nodes: self.nodes,
            }),
            _ => Err(PickleErrorKind::BadFinalStack),
        }
    }
}
