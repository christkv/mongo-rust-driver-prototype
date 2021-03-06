/* Copyright 2013 10gen Inc.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use std::to_bytes::*;
use std::str::count_bytes;
use std::rand::*;
use extra::serialize::*;
use tools::ord_hash::*;
use std::vec::{VecIterator, VecRevIterator};

//TODO: ideally test this on a big-endian machine
#[cfg(target_endian = "little")]
static L_END: bool = true;

#[cfg(target_arch = "big")]
static L_END: bool = false;

/**
 * Algebraic data type representing the BSON AST.
 * BsonDocument maps string keys to this type.
 * This can be converted back and forth from BsonDocument
 * by using the Embedded variant.
 */
#[deriving(Eq,Clone)]
pub enum Document {
    Double(f64),                           //x01
    UString(~str),                         //x02
    Embedded(~BsonDocument),               //x03
    Array(~BsonDocument),                  //x04
    Binary(u8, ~[u8]),                     //x05
    //deprecated: x06 undefined
    ObjectId(~[u8]),                       //x07
    Bool(bool),                            //x08
    UTCDate(i64),                          //x09
    Null,                                  //x0A
    Regex(~str, ~str),                     //x0B
    DBRef(~str, ~Document),                //x0C
    JScript(~str),                         //x0D
    JScriptWithScope(~str, ~BsonDocument), //x0F
    //deprecated: x0E symbol
    Int32(i32),                            //x10
    Timestamp(u32, u32),                   //x11
    Int64(i64),                            //x12
    MinKey,                                //xFF
    MaxKey                                 //x7F

}

/**
 * A factory for constructing ObjectIds.
 *
 * The first 4 bytes of an OID are the number
 * of seconds since the Unix epoch.
 * The next 3 bytes are based on the name of
 * the host machine name.
 * The next 2 bytes are based on the PID of
 * the current process.
 * The final 4 bytes are incrementally generated
 * from a random value.
 */
struct ObjIdFactory {
    rseed: u32,
    time: u32
}

impl ObjIdFactory {

    ///Get a new ObjIdFactory.
    pub fn new() -> ObjIdFactory {
        use extra::time::get_time;
        ObjIdFactory {
            rseed: (&mut XorShiftRng::new()).next() % (get_time().nsec as u32),
            time: get_time().sec as u32
        }
    }

    ///Generate an ObjectId.
    pub fn oid(&mut self) -> Document {
        use tools::md5::md5;
        use std::libc::getpid;

        let mut bytes: ~[u8] = ~[];

        let time = self.time.to_bytes(L_END);
        //TODO: need a gethostname function
        let hostname = md5(~"localhost").to_bytes(L_END);
        let pid = (unsafe { getpid() }).to_bytes(L_END);
        let rand = self.rseed.to_bytes(L_END);

        bytes.push_all(time);
        bytes.push_all(hostname.slice(0,3));
        bytes.push_all(pid.slice(0,2));
        bytes.push_all(rand.slice(0,3));
        self.rseed = (self.rseed + 1) % 0xFFFFFF;

        ObjectId(bytes)
    }
}

/**
* The type of a complete BSON document.
* Contains an ordered map of fields and values and the size of the document as i32.
*/
#[deriving(Eq)]
pub struct BsonDocument {
    size: i32,
    fields: ~OrderedHashmap<~str, Document>
}

/**
 * serialize::Encoder object for Bson.
 * After encoding has been done with an Encoder instance,
 * encoder.buf will contain the resulting ~[u8].
 */
pub struct BsonDocEncoder {
    //XXX: is it possible this could be an IOWriter, like the extra::json encoder?
    priv buf: ~[u8]
}

macro_rules! cstr(
    ($val:ident) => {
        |e| (
            for $val.iter().advance |c| {
                e.emit_char(c);
            }
        )
    }
)

impl Clone for BsonDocument {
    pub fn clone(&self) -> BsonDocument {
        let mut map = ~OrderedHashmap::new();
        for self.fields.iter().advance |&(@k, @v)| {
            map.insert(k, v);
        }
        BsonDocument {
            size: self.size,
            fields: map
        }
    }
}

///serialize::Encoder implementation.
impl Encoder for BsonDocEncoder {
    fn emit_nil(&mut self) { }
    fn emit_uint(&mut self, v: uint) { self.emit_i32(v as i32); }
    fn emit_u8(&mut self, v: u8) { self.buf.push(v) }
    fn emit_u16(&mut self, v: u16) { self.emit_i32(v as i32); }
    fn emit_u32(&mut self, v: u32) { self.emit_i32(v as i32); }
    fn emit_u64(&mut self, v: u64) { self.emit_i64(v as i64); }
    //TODO target architectures with cfg
    fn emit_int(&mut self, v: int) { self.emit_i32(v as i32); }
    fn emit_i64(&mut self, v: i64) {
        self.buf.push_all(v.to_bytes(L_END))
    }
    fn emit_i32(&mut self, v: i32) {
        self.buf.push_all(v.to_bytes(L_END))
    }
    fn emit_i16(&mut self, v: i16) { self.emit_i32(v as i32); }
    fn emit_i8(&mut self, v: i8) { self.emit_i32(v as i32); }
    fn emit_bool(&mut self, v: bool) {
        self.buf.push_all((if v {~[1]} else {~[0]}))
    }
    fn emit_f64(&mut self, v: f64) {
        self.buf.push_all(v.to_bytes(L_END));
    }
    fn emit_f32(&mut self, v: f32) { self.emit_f64(v as f64); }
    fn emit_float(&mut self, v: float) { self.emit_f64(v as f64); }
    fn emit_str(&mut self, v: &str) {
        self.buf.push_all((1 + count_bytes(v, 0, v.len()) as i32).to_bytes(L_END)
            + v.bytes_iter().collect::<~[u8]>() + ~[0u8]);
        }

    fn emit_map_elt_key(&mut self, l: uint, f: &fn(&mut BsonDocEncoder)) {
        if l == 0 { return; } //if the key is empty, return
        f(self);
        self.emit_u8(0u8);
    }
    fn emit_map_elt_val(&mut self, _: uint, f: &fn(&mut BsonDocEncoder)) {
        f(self);
    }
    fn emit_char(&mut self, c: char) { self.buf.push(c as u8); }

    //unimplemented junk
    fn emit_struct(&mut self, _: &str, _: uint, _: &fn(&mut BsonDocEncoder)) { }
    fn emit_enum(&mut self, _: &str, _: &fn(&mut BsonDocEncoder)) { }
    fn emit_enum_variant(&mut self, _: &str, _: uint, _:uint, _:&fn(&mut BsonDocEncoder)) { }
    fn emit_enum_variant_arg(&mut self, _:uint, _:&fn(&mut BsonDocEncoder)) { }
    fn emit_enum_struct_variant(&mut self, _: &str, _: uint, _:uint, _:&fn(&mut BsonDocEncoder)) { }
    fn emit_enum_struct_variant_field(&mut self, _: &str, _:uint, _:&fn(&mut BsonDocEncoder)) { }
    fn emit_struct_field(&mut self, _: &str, _:uint, _:&fn(&mut BsonDocEncoder)) { }
    fn emit_tuple(&mut self, _: uint, _:&fn(&mut BsonDocEncoder)) { }
    fn emit_tuple_arg(&mut self, _:uint, _:&fn(&mut BsonDocEncoder)) { }
    fn emit_tuple_struct(&mut self, _: &str, _:uint, _:&fn(&mut BsonDocEncoder)) { }
    fn emit_tuple_struct_arg(&mut self, _:uint, _:&fn(&mut BsonDocEncoder)) { }
    fn emit_option(&mut self, _:&fn(&mut BsonDocEncoder)) { }
    fn emit_option_none(&mut self) { }
    fn emit_option_some(&mut self, _: &fn(&mut BsonDocEncoder)) { }
    fn emit_seq(&mut self, _: uint, _: &fn(&mut BsonDocEncoder)) { }
    fn emit_seq_elt(&mut self, _: uint, _: &fn(&mut BsonDocEncoder)) { }
    fn emit_map(&mut self, _: uint, _: &fn(&mut BsonDocEncoder)) { }


}

///Encoder for BsonDocuments
impl BsonDocEncoder {
    fn new() -> BsonDocEncoder { BsonDocEncoder { buf: ~[] } }
}

///Light wrapper around a typical Map implementation.
impl<E:Encoder> Encodable<E> for BsonDocument {
    fn encode(&self, encoder: &mut E) {
        encoder.emit_i32(self.size);
        for self.fields.iter().advance |&(@k, @v)| {
            let b = match v {
               Double(_) => 0x01,
               UString(_) => 0x02,
               Embedded(_) => 0x03,
               Array(_) => 0x04,
               Binary(_,_) => 0x05,
               ObjectId(_) => 0x07,
               Bool(_) => 0x08,
               UTCDate(_) => 0x09,
               Null => 0x0A,
               Regex(_,_) => 0x0B,
               DBRef(_,_) => 0x0C,
               JScript(_) => 0x0D,
               JScriptWithScope(_,_) => 0x0F,
               Int32(_) => 0x10,
               Timestamp(_,_) => 0x11,
               Int64(_) => 0x12,
               MinKey => 0xFF,
               MaxKey => 0x7F
            };

            encoder.emit_u8(b);
            encoder.emit_map_elt_key(k.len(), cstr!(k));
            do encoder.emit_map_elt_val(0) |e| {
                v.encode(e);
            }
        }
        encoder.emit_u8(0u8);
    }
}
///Encodable implementation for Document.
impl<E:Encoder> Encodable<E> for Document {
    ///After encode is run, the field 'buf' in the Encoder object will contain the encoded value.
    ///See bson_types.rs:203
    fn encode(&self, encoder: &mut E) {
        match *self {
            Double(f) => {
                encoder.emit_f64(f as f64);
            }
            UString(ref s) => {
                encoder.emit_str(*s);
            }
            Embedded(ref doc) => {
                doc.encode(encoder);
            }
            Array(ref doc) => {
                doc.encode(encoder);
            }
            Binary(st, ref dat) => {
                encoder.emit_i32(dat.len() as i32);
                encoder.emit_u8(st);
                for dat.iter().advance |&elt| {
                    encoder.emit_u8(elt);
                }
            }
            ObjectId(ref id) => {
                if !(id.len() == 12) {
                    fail!(fmt!("invalid object id found: %?", id));
                }
                for id.iter().advance |&elt| {
                    encoder.emit_u8(elt);
                }
            }
            Bool(b) => {
                encoder.emit_bool(b);
            }
            UTCDate(i) => {
                encoder.emit_i64(i);
            }
            Null => { }
            Regex(ref s1, ref s2) => {
                encoder.emit_map_elt_val(0, cstr!(s1));
                encoder.emit_u8(0u8);
                encoder.emit_map_elt_val(0, cstr!(s2));
                encoder.emit_u8(0u8);
            }
            DBRef(ref s, ref doc) => {
                encoder.emit_str(*s);
                doc.encode(encoder);
            }
            JScript(ref s) => {
                encoder.emit_str(*s);
            }
            JScriptWithScope(ref s, ref doc) => {
                encoder.emit_i32(5 + doc.size + (s.to_bytes(L_END).len() as i32));
                encoder.emit_map_elt_val(0, cstr!(s));
                encoder.emit_u8(0u8);
                doc.encode(encoder);
            }
            Int32(i) => {
                encoder.emit_i32(i);
            }
            Timestamp(u1, u2) => {
                encoder.emit_u32(u1);
                encoder.emit_u32(u2);
            }
            Int64(i) => {
                encoder.emit_i64(i); }
            MinKey => { }
            MaxKey => { }
        }
    }
}

impl ToStr for BsonDocument {
    pub fn to_str(&self) -> ~str {
        self.fields.to_str()
    }
}

impl<'self> BsonDocument {

    ///Convert this document to its binary BSON representation.
    pub fn to_bson(&self) -> ~[u8] {
        let mut encoder = BsonDocEncoder::new();
        self.encode(&mut encoder);
        encoder.buf //the encoded value is contained here
    }

    ///Get a forwards iterator for this document.
    pub fn iter(&'self self) -> VecIterator<'self, (@~str, @Document)> {
        self.fields.iter()
    }

    ///Get a reverse iterator for this document.
    pub fn rev_iter(&'self self) -> VecRevIterator<'self, (@~str, @Document)> {
        self.fields.rev_iter()
    }

    ///Check if this document contains the given key.
    pub fn contains_key(&self, key: ~str) -> bool {
        self.fields.contains_key(&key)
    }

    ///Find the value for the given key, if it exists.
    pub fn find<'a>(&'a self, key: ~str) -> Option<&'a Document> {
        self.fields.find(&key)
    }

    ///Adds a key/value pair and updates size appropriately. Returns nothing.
    pub fn put(&mut self, key: ~str, val: Document) {
        self.fields.insert(key, val);
        self.size = map_size(self.fields);
    }

    ///Adds a list of key/value pairs and updates size. Returns nothing.
    pub fn put_all(&mut self, pairs: ~[(~str, Document)]) {
        for pairs.iter().advance |&(k,v)| {
            self.fields.insert(k, v);
        }
        self.size = map_size(self.fields);
    }

    /**
     * If the provided document is Embedded, puts all of its (key,val) pairs
     * into self. Otherwise, does nothing.
     */
    pub fn union(&mut self, other : Document) {
        match other {
            Embedded(doc) => {
                for doc.fields.iter().advance |&(@k,@v)| {
                    self.put(k,v);
                }
            }
            _ => (),
        }
    }

    ///Returns a new BsonDocument struct.
    ///The default size is 5: 4 for the size integer and 1 for the terminating 0x0.
    pub fn new() -> BsonDocument {
        BsonDocument { size: 5, fields: ~OrderedHashmap::new() }
    }

    /**
    * Compare two BsonDocuments to decide if they have the same fields.
    * Returns true if every field except the _id field is matching.
    * The _id field and the size are ignored.
    * Two documents are considered to have matching fields even if
    * their fields are not in the same order.
    */
    pub fn fields_match(&self, other: &BsonDocument) -> bool {
        let mut b: bool = true;
        for self.fields.iter().advance |&(@key, @val)| {
            if !(key==~"_id") {
                let mut found_match = false;
                for other.fields.iter().advance |&(@okey, @oval)| {
                    found_match |= ((key==okey)&&(val==oval));
                }
                b &= found_match;
            }
        }
        b
    }

    fn from_map(m: ~OrderedHashmap<~str, Document>) -> BsonDocument {
        BsonDocument { size: map_size(m), fields: m }
    }

}

///Methods on documents.
impl Document {

    ///Allows any document to be converted to its BSON-serialized representation.
    pub fn to_bson(&self) -> ~[u8] {
        let mut encoder = BsonDocEncoder::new();
        self.encode(&mut encoder);
        encoder.buf
    }

    ///Reports the size of a document's BSON representation.
    fn size(&self) -> i32 {
        match *self {
            Double(_) => 8,
            UString(ref s) => 5 + (*s).to_bytes(L_END).len() as i32,
            Embedded(ref doc) => doc.size,
            Array(ref doc) => doc.size,
            Binary(_, ref dat) => 5 + dat.len() as i32,
            ObjectId(_) => 12,
            Bool(_) => 1,
            UTCDate(_) => 8,
            Null => 0,
            Regex(ref s1, ref s2) => 2 + (s1.to_bytes(L_END).len() + s2.to_bytes(L_END).len()) as i32,
            DBRef(ref s, _) => 17 + (*s).to_bytes(L_END).len() as i32, //doc had to be a 12-byte oid
            JScript(ref s) => 5 + (*s).to_bytes(L_END).len() as i32,
            JScriptWithScope(ref s, ref doc) => 5 + (*s).to_bytes(L_END).len() as i32 + doc.size,
            Int32(_) => 4,
            Timestamp(_,_) => 8,
            Int64(_) => 8,
            MinKey => 0,
            MaxKey => 0
        }
    }
}

impl ToStr for Document {
    pub fn to_str(&self) -> ~str {
        match *self {
            Double(f) => f.to_str(),
            UString(ref s) => s.clone(),
            Embedded(ref doc) => doc.to_str(),
            Array(ref doc) => doc.to_str(),
            Binary(st, ref dat) => fmt!("Binary(%s, %s)", st.to_str(), dat.to_str()),
            ObjectId(ref d) => {
                let mut s = ~"";
                for d.iter().advance |&b| {
                    let mut x = fmt!("%x",b as uint);
                    if x.len() == 1 {
                        x = (~"0").append(x);
                    }
                    s.push_str(x);
                }
                s
            }
            Bool(b) => b.to_str(),
            UTCDate(d) => d.to_str(), //TODO actually format this
            Null => ~"null",
            Regex(ref s1, ref s2) => fmt!("Regex(%s, %s)", *s1, *s2),
            DBRef(ref s, ref doc) => fmt!("DBRef(%s, %s)", *s, doc.to_str()),
            JScript(ref s) => s.clone(),
            JScriptWithScope(ref s, ref doc) => fmt!("JScope(%s, %s)", *s, doc.to_str()),
            Int32(i) => i.to_str(),
            Timestamp(u1,u2) => fmt!("Timestamp(%s, %s)", u1.to_str(), u2.to_str()),
            Int64(i) => i.to_str(),
            MinKey => ~"minKey",
            MaxKey => ~"maxKey"
        }
    }
}

//Calculate the size of a BSON object based on its fields.
priv fn map_size(m: &OrderedHashmap<~str, Document>)  -> i32{
    let mut sz: i32 = 4; //since this map is going in an object, it has a 4-byte size variable
    for m.iter().advance |&(k, v)| {
        sz += (k.to_bytes(L_END).len() as i32) + v.size() + 2; //1 byte format code, trailing 0 after each key
    }
    sz + 1 //trailing 0 byte
}

#[cfg(test)]
mod tests {
    use super::*;
    use json_parse::*;
    use std::rand::Rng;
    use std::rand;
    use extra::test::BenchHarness;

    //testing size computation
    #[test]
    fn test_obj_size() {
        let mut doc1 = BsonDocument::new();
        doc1.put(~"0", UString(~"hello"));
        doc1.put(~"1", Bool(false));

        assert_eq!(doc1.size, 22);

        let mut doc2 = BsonDocument::new();
        doc2.put(~"foo", UString(~"bar"));
        doc2.put(~"baz", UString(~"qux"));
        doc2.put(~"doc", Embedded(~doc1));

        assert_eq!(doc2.size, 58);
    }

    #[test]
    fn test_double_encode() {
        let mut doc = BsonDocument::new();
        doc.put(~"foo", Double(3.14159f64));
        assert_eq!(doc.to_bson(), ~[18,0,0,0,1,102,111,111,0,110,134,27,240,249,33,9,64,0]);
    }
    #[test]
    fn test_string_encode() {
        let mut doc = BsonDocument::new();
        doc.put(~"foo", UString(~"bar"));
        assert_eq!(doc.to_bson(), ~[18,0,0,0,2,102,111,111,0,4,0,0,0,98,97,114,0,0]);
    }

    #[test]
    fn test_bool_encode() {
        let mut doc = BsonDocument::new();
        doc.put(~"foo", Bool(true));
        assert_eq!(doc.to_bson(), ~[11,0,0,0,8,102,111,111,0,1,0] );
    }

    #[test]
    fn test_32bit_encode() {
        let mut doc = BsonDocument::new();
        doc.put(~"foo", Int32(56 as i32));
        assert_eq!(doc.to_bson(), ~[14,0,0,0,16,102,111,111,0,56,0,0,0,0]);
    }

    #[test]
    fn test_embed_encode() {
        //lists
        let mut inside = BsonDocument::new();
        inside.put_all(~[(~"0", UString(~"hello")), (~"1", Bool(false))]);
        let mut doc2 = BsonDocument::new();
        doc2.put_all(~[(~"foo", Array(~inside.clone())), (~"baz", UString(~"qux"))]);

        assert_eq!(doc2.to_bson(), ~[45,0,0,0,4,102,111,111,0,22,0,0,0,2,48,0,6,0,0,0,104,101,108,108,111,0,8,49,0,0,0,2,98,97,122,0,4,0,0,0,113,117,120,0,0]);

        //embedded objects
        let mut doc3 = BsonDocument::new();
        doc3.put_all(~[(~"foo", Embedded(~inside.clone())), (~"baz", UString(~"qux"))]);

        assert_eq!(doc3.to_bson(), ~[45,0,0,0,3,102,111,111,0,22,0,0,0,2,48,0,6,0,0,0,104,101,108,108,111,0,8,49,0,0,0,2,98,97,122,0,4,0,0,0,113,117,120,0,0]);

        let mut doc4 = BsonDocument::new();
        doc4.put_all(~[(~"foo", JScriptWithScope(~"wat", ~inside.clone())), (~"baz", UString(~"qux"))]);
        assert_eq!(doc4.to_bson(), ~[53,0,0,0,15,102,111,111,0,30,0,0,0,119,97,116,0,22,0,0,0,2,48,0,6,0,0,0,104,101,108,108,111,0,8,49,0,0,0,2,98,97,122,0,4,0,0,0,113,117,120,0,0]);


    }

    #[test]
    fn test_binary_encode() {
        let mut doc = BsonDocument::new();
        doc.put(~"foo", Binary(2u8, ~[0u8,1,2,3]));
        assert_eq!(doc.to_bson(), ~[19,0,0,0,5,102,111,111,0,4,0,0,0,2,0,1,2,3,0]);
    }
    #[test]
    fn test_64bit_encode() {
        let mut doc1 = BsonDocument::new();
        doc1.put(~"foo", UTCDate(4040404 as i64));
        assert_eq!(doc1.to_bson(), ~[18,0,0,0,9,102,111,111,0,212,166,61,0,0,0,0,0,0] );

        let mut doc2 = BsonDocument::new();
        doc2.put(~"foo", Int64(4040404 as i64));
        assert_eq!(doc2.to_bson(), ~[18,0,0,0,18,102,111,111,0,212,166,61,0,0,0,0,0,0] );

        let mut doc3 = BsonDocument::new();
        doc3.put(~"foo", Timestamp(4040404, 0));
        assert_eq!(doc3.to_bson(), ~[18,0,0,0,17,102,111,111,0,212,166,61,0,0,0,0,0,0] );
    }

    #[test]
    fn test_null_encode() {
        let mut doc = BsonDocument::new();
        doc.put(~"foo", Null);

        assert_eq!(doc.to_bson(), ~[10,0,0,0,10,102,111,111,0,0]);
    }

    #[test]
    fn test_regex_encode() {
        let mut doc = BsonDocument::new();
        doc.put(~"foo", Regex(~"bar", ~"baz"));

        assert_eq!(doc.to_bson(), ~[18,0,0,0,11,102,111,111,0,98,97,114,0,98,97,122,0,0]);
    }

    #[test]
    fn test_dbref_encode() {
        let mut doc = BsonDocument::new();
        doc.put(~"foo", DBRef(~"bar", ~ObjectId(~[0u8,1,2,3,4,5,6,7,8,9,10,11])));
        assert_eq!(doc.to_bson(), ~[30,0,0,0,12,102,111,111,0,4,0,0,0,98,97,114,0,0,1,2,3,4,5,6,7,8,9,10,11,0])
    }

    #[test]
    fn test_jscript_encode() {
        let mut doc = BsonDocument::new();
        doc.put(~"foo", JScript(~"return 1;"));
        assert_eq!(doc.to_bson(), ~[24,0,0,0,13,102,111,111,0,10,0,0,0,114,101,116,117,114,110,32,49,59,0,0]);
    }
    #[test]
    fn test_valid_objid_encode() {
        let mut doc = BsonDocument::new();
        doc.put(~"foo", ObjectId(~[0,1,2,3,4,5,6,7,8,9,10,11]));

        assert_eq!(doc.to_bson(), ~[22,0,0,0,7,102,111,111,0,0,1,2,3,4,5,6,7,8,9,10,11,0]);
    }

    #[test]
    #[should_fail]
    fn test_invalid_objid_encode() {
        let mut doc = BsonDocument::new();
        doc.put(~"foo", ObjectId(~[1,2,3]));
        doc.to_bson();
    }

    #[test]
    fn test_multi_encode() {
        let mut doc = BsonDocument::new();
        doc.put(~"foo", Bool(true));
        doc.put(~"bar", UString(~"baz"));
        doc.put(~"qux", Int32(404));

        let enc = doc.to_bson();

        assert_eq!(enc, ~[33,0,0,0,8,102,111,111,0,1,2,98,97,114,0,4,0,0,0,98,97,122,0,16,113,117,120,0,148,1,0,0,0]);
    }

    //full encode path testing
    #[test]
    fn test_string_whole_encode() {
        let mut doc = BsonDocument::new();
        doc.put(~"foo", UString(~"bar"));

        assert_eq!(doc.to_bson(), ~[18,0,0,0,2,102,111,111,0,4,0,0,0,98,97,114,0,0]);
    }

    #[test]
    fn test_embed_whole_encode() {
        let jstring = "{\"foo\": [\"hello\", false], \"baz\": \"qux\"}";
        let doc = match ObjParser::from_string::<Document, ExtendedJsonParser<~[char]>>(jstring).unwrap() {
            Embedded(ref map) => BsonDocument::from_map(map.fields.clone()),
            _ => fail!("test_embed_whole_encode parse failure")
        };

        assert_eq!(doc.to_bson(), ~[45,0,0,0,4,102,111,111,0,22,0,0,0,2,48,0,6,0,0,0,104,101,108,108,111,0,8,49,0,0,0,2,98,97,122,0,4,0,0,0,113,117,120,0,0]);
    }

    #[bench]
    fn bench_val_encode(b: &mut BenchHarness) {
        let seed = [1,2,3,4,5,6,7,8,9,0];
        let mut rand = rand::IsaacRng::new_seeded(seed);
        let mut doc = BsonDocument::new();
        do b.iter {
            doc.put(~"foo", Double(rand.next() as f64));
            doc.put(~"bar", Double(rand.next() as f64));
            doc.put(~"baz", Double(rand.next() as f64));
            doc.to_bson();
            doc = BsonDocument::new();
        }
    }

    #[bench]
    fn bench_advanced_object(b: &mut BenchHarness) {
        let stream = "{
            'fullName' : 'John Doe',
            'age' : 42,
            'state' : 'Massachusetts',
            'city' : 'Boston',
            'zip' : 02201,
            'married' : false,
            'dozen' : 12,
            'topThreeFavoriteColors' : [ 'red', 'magenta', 'cyan' ],
            'favoriteSingleDigitWholeNumbers' : [ 7 ],
            'favoriteFiveLetterWord' : 'fadsy',
            'strings' :
            [
            'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ',
            '01234567890',
            'mixed-1234-in-{+^}',
            '\"quoted\"',
            '\"\\e\\s\\c\\a\\p\\e\\d\"',
            '\"quoted-at-sign@sld.org\"',
            '\"escaped\\\"quote\"',
            '\"back\\slash\"',
            'email@address.com'
            ],
            'ipAddresses' : [ '127.0.0.1', '24.48.64.2', '192.168.1.1', '209.68.44.3', '2.2.2.2' ]
        }".iter().collect::<~[char]>();

        let mut parser = ExtendedJsonParser::new(stream.clone());
        let doc = match parser.object() {
            Ok(d) => d,
            Err(e) => fail!(e.to_str())
        };
        do b.iter {
            doc.to_bson();
        }
    }

    #[bench]
    fn bench_extended_object(b: &mut BenchHarness) {
        let stream = "{
            'name': 'foo',
            'baz': 'qux',
            'binary': { '$binary': 012345432, '$type': 0 },
            'dates': [ { '$date': 987654 }, {'$date': 123456}, {'$date': 748392} ],
            'timestamp': { 'timestamp': { 'timestamp': { '$timestamp': { 't': 1234, 'i': 5678 } } } },
            'regex': { '$regex': '^.*/012345/.*(foo|bar)+.*$', '$options': '-j -g -i' },
            'oid': { '$oid': 43214321 },
            'minkey': { 'maxkey': { 'that-was-a-fakeout': { '$minKey': 1 } } },
            'maxkey': { 'minkey': { 'haha-that-too': { '$maxKey': 1 } } }
        }".iter().collect::<~[char]>();

        let mut parser = ExtendedJsonParser::new(stream.clone());
        let doc = match parser.object() {
            Ok(d) => d,
            Err(e) => fail!(e.to_str())
        };
        do b.iter {
            doc.to_bson();
        }
    }
}
