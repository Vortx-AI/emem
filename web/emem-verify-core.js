/* emem receipt verification, self-contained.
 *
 * One algorithm, two deliveries. emem.dev/verify loads this file over HTTP;
 * the MCP Apps fact card (ui://emem/fact-card) has it compiled in, because a
 * SEP-1865 host applies `connect-src 'none'` by default and a card that
 * fetched a hash library at runtime would fail silently and draw a broken
 * panel.
 *
 * The encoders below mirror emem-attest and emem-fact. They are not trusted
 * on that claim: `ememSelfTest()` replays vectors emitted by the Rust signer
 * and every caller MUST refuse to report a verification result when it fails.
 * A wrong encoder can only ever produce a wrong digest, and a wrong digest
 * reads as "tampered" against a receipt that is perfectly sound. Telling a
 * user their genuine fact was forged is a worse failure than declining to
 * check it, so the self-test gates the whole surface.
 *
 * Crypto is @noble/hashes 1.5.0 (blake3) and @noble/curves 1.6.0 (ed25519),
 * bundled with esbuild. Both are checked against the official BLAKE3 vectors
 * and an ed25519 round trip by scripts/verify_core_test.cjs.
 */
(function (root) {
"use strict";

// ── vendored crypto (esbuild bundle) ──
(()=>{function ft(t){if(!Number.isSafeInteger(t)||t<0)throw new Error(`positive integer expected, not ${t}`)}function Ge(t){return t instanceof Uint8Array||t!=null&&typeof t=="object"&&t.constructor.name==="Uint8Array"}function mt(t,...e){if(!Ge(t))throw new Error("Uint8Array expected");if(e.length>0&&!e.includes(t.length))throw new Error(`Uint8Array expected of length ${e}, not of length=${t.length}`)}function et(t,e=!0){if(t.destroyed)throw new Error("Hash instance has been destroyed");if(e&&t.finished)throw new Error("Hash#digest() has already been called")}function pt(t,e){mt(t);let n=e.outputLen;if(t.length<n)throw new Error(`digestInto() expects output buffer of length at least ${n}`)}var At=BigInt(4294967295),vt=BigInt(32);function Bt(t,e=!1){return e?{h:Number(t&At),l:Number(t>>vt&At)}:{h:Number(t>>vt&At)|0,l:Number(t&At)|0}}function Xe(t,e=!1){let n=new Uint32Array(t.length),r=new Uint32Array(t.length);for(let s=0;s<t.length;s++){let{h:o,l:a}=Bt(t[s],e);[n[s],r[s]]=[o,a]}return[n,r]}var je=(t,e)=>BigInt(t>>>0)<<vt|BigInt(e>>>0),Pe=(t,e,n)=>t>>>n,Ye=(t,e,n)=>t<<32-n|e>>>n,We=(t,e,n)=>t>>>n|e<<32-n,Ke=(t,e,n)=>t<<32-n|e>>>n,Qe=(t,e,n)=>t<<64-n|e>>>n-32,Je=(t,e,n)=>t>>>n-32|e<<64-n,tn=(t,e)=>e,en=(t,e)=>t,nn=(t,e,n)=>t<<n|e>>>32-n,on=(t,e,n)=>e<<n|t>>>32-n,rn=(t,e,n)=>e<<n-32|t>>>64-n,sn=(t,e,n)=>t<<n-32|e>>>64-n;function cn(t,e,n,r){let s=(e>>>0)+(r>>>0);return{h:t+n+(s/2**32|0)|0,l:s|0}}var fn=(t,e,n)=>(t>>>0)+(e>>>0)+(n>>>0),an=(t,e,n,r)=>e+n+r+(t/2**32|0)|0,un=(t,e,n,r)=>(t>>>0)+(e>>>0)+(n>>>0)+(r>>>0),hn=(t,e,n,r,s)=>e+n+r+s+(t/2**32|0)|0,ln=(t,e,n,r,s)=>(t>>>0)+(e>>>0)+(n>>>0)+(r>>>0)+(s>>>0),dn=(t,e,n,r,s,o)=>e+n+r+s+o+(t/2**32|0)|0;var pn={fromBig:Bt,split:Xe,toBig:je,shrSH:Pe,shrSL:Ye,rotrSH:We,rotrSL:Ke,rotrBH:Qe,rotrBL:Je,rotr32H:tn,rotr32L:en,rotlSH:nn,rotlSL:on,rotlBH:rn,rotlBL:sn,add:cn,add3L:fn,add3H:an,add4L:un,add4H:hn,add5H:dn,add5L:ln},y=pn;var at=typeof globalThis=="object"&&"crypto"in globalThis?globalThis.crypto:void 0;var ce=t=>new Uint8Array(t.buffer,t.byteOffset,t.byteLength),ut=t=>new Uint32Array(t.buffer,t.byteOffset,Math.floor(t.byteLength/4)),St=t=>new DataView(t.buffer,t.byteOffset,t.byteLength),Et=(t,e)=>t<<32-e|t>>>e;var q=new Uint8Array(new Uint32Array([287454020]).buffer)[0]===68,fe=t=>t<<24&4278190080|t<<8&16711680|t>>>8&65280|t>>>24&255,qt=q?t=>t:t=>fe(t);function D(t){for(let e=0;e<t.length;e++)t[e]=fe(t[e])}function ae(t){if(typeof t!="string")throw new Error(`utf8ToBytes expected string, got ${typeof t}`);return new Uint8Array(new TextEncoder().encode(t))}function Q(t){return typeof t=="string"&&(t=ae(t)),mt(t),t}var xt=class{clone(){return this._cloneInto()}},Yn={}.toString;function ue(t){let e=r=>t().update(Q(r)).digest(),n=t();return e.outputLen=n.outputLen,e.blockLen=n.blockLen,e.create=()=>t(),e}function he(t){let e=(r,s)=>t(s).update(Q(r)).digest(),n=t({});return e.outputLen=n.outputLen,e.blockLen=n.blockLen,e.create=r=>t(r),e}function le(t=32){if(at&&typeof at.getRandomValues=="function")return at.getRandomValues(new Uint8Array(t));if(at&&typeof at.randomBytes=="function")return at.randomBytes(t);throw new Error("crypto.getRandomValues must be defined")}var Lt=class extends xt{constructor(e,n,r={},s,o,a){if(super(),this.blockLen=e,this.outputLen=n,this.length=0,this.pos=0,this.finished=!1,this.destroyed=!1,ft(e),ft(n),ft(s),n<0||n>s)throw new Error("outputLen bigger than keyLen");if(r.key!==void 0&&(r.key.length<1||r.key.length>s))throw new Error(`key must be up 1..${s} byte long or undefined`);if(r.salt!==void 0&&r.salt.length!==o)throw new Error(`salt must be ${o} byte long or undefined`);if(r.personalization!==void 0&&r.personalization.length!==a)throw new Error(`personalization must be ${a} byte long or undefined`);this.buffer32=ut(this.buffer=new Uint8Array(e))}update(e){et(this);let{blockLen:n,buffer:r,buffer32:s}=this;e=Q(e);let o=e.length,a=e.byteOffset,c=e.buffer;for(let i=0;i<o;){this.pos===n&&(q||D(s),this.compress(s,0,!1),q||D(s),this.pos=0);let u=Math.min(n-this.pos,o-i),f=a+i;if(u===n&&!(f%4)&&i+u<o){let h=new Uint32Array(c,f,Math.floor((o-i)/4));q||D(h);for(let p=0;i+n<o;p+=s.length,i+=n)this.length+=n,this.compress(h,p,!1);q||D(h);continue}r.set(e.subarray(i,i+u),this.pos),this.pos+=u,this.length+=u,i+=u}return this}digestInto(e){et(this),pt(e,this);let{pos:n,buffer32:r}=this;this.finished=!0,this.buffer.subarray(n).fill(0),q||D(r),this.compress(r,0,!0),q||D(r);let s=ut(e);this.get().forEach((o,a)=>s[a]=qt(o))}digest(){let{buffer:e,outputLen:n}=this;this.digestInto(e);let r=e.slice(0,n);return this.destroy(),r}_cloneInto(e){let{buffer:n,length:r,finished:s,destroyed:o,outputLen:a,pos:c}=this;return e||(e=new this.constructor({dkLen:a})),e.set(...this.get()),e.length=r,e.finished=s,e.destroyed=o,e.outputLen=a,e.buffer.set(n),e.pos=c,e}};var W=new Uint32Array([1779033703,3144134277,1013904242,2773480762,1359893119,2600822924,528734635,1541459225]);function nt(t,e,n,r,s){return t=t+e+s|0,r=Et(r^t,16),n=n+r|0,e=Et(e^n,12),{a:t,b:e,c:n,d:r}}function ot(t,e,n,r,s){return t=t+e+s|0,r=Et(r^t,8),n=n+r|0,e=Et(e^n,7),{a:t,b:e,c:n,d:r}}function Dt(t,e,n,r,s,o,a,c,i,u,f,h,p,b,E,g,I,S,k,d){let B=0;for(let H=0;H<r;H++)({a:s,b:i,c:p,d:I}=nt(s,i,p,I,n[e+t[B++]])),{a:s,b:i,c:p,d:I}=ot(s,i,p,I,n[e+t[B++]]),{a:o,b:u,c:b,d:S}=nt(o,u,b,S,n[e+t[B++]]),{a:o,b:u,c:b,d:S}=ot(o,u,b,S,n[e+t[B++]]),{a,b:f,c:E,d:k}=nt(a,f,E,k,n[e+t[B++]]),{a,b:f,c:E,d:k}=ot(a,f,E,k,n[e+t[B++]]),{a:c,b:h,c:g,d}=nt(c,h,g,d,n[e+t[B++]]),{a:c,b:h,c:g,d}=ot(c,h,g,d,n[e+t[B++]]),{a:s,b:u,c:E,d}=nt(s,u,E,d,n[e+t[B++]]),{a:s,b:u,c:E,d}=ot(s,u,E,d,n[e+t[B++]]),{a:o,b:f,c:g,d:I}=nt(o,f,g,I,n[e+t[B++]]),{a:o,b:f,c:g,d:I}=ot(o,f,g,I,n[e+t[B++]]),{a,b:h,c:p,d:S}=nt(a,h,p,S,n[e+t[B++]]),{a,b:h,c:p,d:S}=ot(a,h,p,S,n[e+t[B++]]),{a:c,b:i,c:b,d:k}=nt(c,i,b,k,n[e+t[B++]]),{a:c,b:i,c:b,d:k}=ot(c,i,b,k,n[e+t[B++]]);return{v0:s,v1:o,v2:a,v3:c,v4:i,v5:u,v6:f,v7:h,v8:p,v9:b,v10:E,v11:g,v12:I,v13:S,v14:k,v15:d}}var de=(()=>{let t=Array.from({length:16},(r,s)=>s),e=r=>[2,6,3,10,7,0,4,13,1,11,12,5,9,14,15,8].map(s=>r[s]),n=[];for(let r=0,s=t;r<7;r++,s=e(s))n.push(...s);return Uint8Array.from(n)})(),Mt=class t extends Lt{constructor(e={},n=0){if(super(64,e.dkLen===void 0?32:e.dkLen,{},Number.MAX_SAFE_INTEGER,0,0),this.flags=0,this.chunkPos=0,this.chunksDone=0,this.stack=[],this.posOut=0,this.bufferOut32=new Uint32Array(16),this.chunkOut=0,this.enableXOF=!0,this.outputLen=e.dkLen===void 0?32:e.dkLen,ft(this.outputLen),e.key!==void 0&&e.context!==void 0)throw new Error("Blake3: only key or context can be specified at same time");if(e.key!==void 0){let r=Q(e.key).slice();if(r.length!==32)throw new Error("Blake3: key should be 32 byte");this.IV=ut(r),q||D(this.IV),this.flags=n|16}else if(e.context!==void 0){let r=new t({dkLen:32},32).update(e.context).digest();this.IV=ut(r),q||D(this.IV),this.flags=n|64}else this.IV=W.slice(),this.flags=n;this.state=this.IV.slice(),this.bufferOut=ce(this.bufferOut32)}get(){return[]}set(){}b2Compress(e,n,r,s=0){let{state:o,pos:a}=this,{h:c,l:i}=Bt(BigInt(e),!0),{v0:u,v1:f,v2:h,v3:p,v4:b,v5:E,v6:g,v7:I,v8:S,v9:k,v10:d,v11:B,v12:H,v13:Z,v14:G,v15:M}=Dt(de,s,r,7,o[0],o[1],o[2],o[3],o[4],o[5],o[6],o[7],W[0],W[1],W[2],W[3],c,i,a,n);o[0]=u^S,o[1]=f^k,o[2]=h^d,o[3]=p^B,o[4]=b^H,o[5]=E^Z,o[6]=g^G,o[7]=I^M}compress(e,n=0,r=!1){let s=this.flags;if(this.chunkPos||(s|=1),(this.chunkPos===15||r)&&(s|=2),r||(this.pos=this.blockLen),this.b2Compress(this.chunksDone,s,e,n),this.chunkPos+=1,this.chunkPos===16||r){let o=this.state;this.state=this.IV.slice();for(let a,c=this.chunksDone+1;(r||!(c&1))&&(a=this.stack.pop());c>>=1)this.buffer32.set(a,0),this.buffer32.set(o,8),this.pos=this.blockLen,this.b2Compress(0,this.flags|4,this.buffer32,0),o=this.state,this.state=this.IV.slice();this.chunksDone++,this.chunkPos=0,this.stack.push(o)}this.pos=0}_cloneInto(e){e=super._cloneInto(e);let{IV:n,flags:r,state:s,chunkPos:o,posOut:a,chunkOut:c,stack:i,chunksDone:u}=this;return e.state.set(s.slice()),e.stack=i.map(f=>Uint32Array.from(f)),e.IV.set(n),e.flags=r,e.chunkPos=o,e.chunksDone=u,e.posOut=a,e.chunkOut=c,e.enableXOF=this.enableXOF,e.bufferOut32.set(this.bufferOut32),e}destroy(){this.destroyed=!0,this.state.fill(0),this.buffer32.fill(0),this.IV.fill(0),this.bufferOut32.fill(0);for(let e of this.stack)e.fill(0)}b2CompressOut(){let{state:e,pos:n,flags:r,buffer32:s,bufferOut32:o}=this,{h:a,l:c}=Bt(BigInt(this.chunkOut++));q||D(s);let{v0:i,v1:u,v2:f,v3:h,v4:p,v5:b,v6:E,v7:g,v8:I,v9:S,v10:k,v11:d,v12:B,v13:H,v14:Z,v15:G}=Dt(de,0,s,7,e[0],e[1],e[2],e[3],e[4],e[5],e[6],e[7],W[0],W[1],W[2],W[3],c,a,n,r);o[0]=i^I,o[1]=u^S,o[2]=f^k,o[3]=h^d,o[4]=p^B,o[5]=b^H,o[6]=E^Z,o[7]=g^G,o[8]=e[0]^I,o[9]=e[1]^S,o[10]=e[2]^k,o[11]=e[3]^d,o[12]=e[4]^B,o[13]=e[5]^H,o[14]=e[6]^Z,o[15]=e[7]^G,q||(D(s),D(o)),this.posOut=0}finish(){if(this.finished)return;this.finished=!0,this.buffer.fill(0,this.pos);let e=this.flags|8;this.stack.length?(e|=4,q||D(this.buffer32),this.compress(this.buffer32,0,!0),q||D(this.buffer32),this.chunksDone=0,this.pos=this.blockLen):e|=(this.chunkPos?0:1)|2,this.flags=e,this.b2CompressOut()}writeInto(e){et(this,!1),mt(e),this.finish();let{blockLen:n,bufferOut:r}=this;for(let s=0,o=e.length;s<o;){this.posOut>=n&&this.b2CompressOut();let a=Math.min(n-this.posOut,o-s);e.set(r.subarray(this.posOut,this.posOut+a),s),this.posOut+=a,s+=a}return e}xofInto(e){if(!this.enableXOF)throw new Error("XOF is not possible after digest call");return this.writeInto(e)}xof(e){return ft(e),this.xofInto(new Uint8Array(e))}digestInto(e){if(pt(e,this),this.finished)throw new Error("digest() was already called");return this.enableXOF=!1,this.writeInto(e),this.destroy(),e}digest(){return this.digestInto(new Uint8Array(this.outputLen))}},pe=he(t=>new Mt(t));function xn(t,e,n,r){if(typeof t.setBigUint64=="function")return t.setBigUint64(e,n,r);let s=BigInt(32),o=BigInt(4294967295),a=Number(n>>s&o),c=Number(n&o),i=r?4:0,u=r?0:4;t.setUint32(e+i,a,r),t.setUint32(e+u,c,r)}var Ot=class extends xt{constructor(e,n,r,s){super(),this.blockLen=e,this.outputLen=n,this.padOffset=r,this.isLE=s,this.finished=!1,this.length=0,this.pos=0,this.destroyed=!1,this.buffer=new Uint8Array(e),this.view=St(this.buffer)}update(e){et(this);let{view:n,buffer:r,blockLen:s}=this;e=Q(e);let o=e.length;for(let a=0;a<o;){let c=Math.min(s-this.pos,o-a);if(c===s){let i=St(e);for(;s<=o-a;a+=s)this.process(i,a);continue}r.set(e.subarray(a,a+c),this.pos),this.pos+=c,a+=c,this.pos===s&&(this.process(n,0),this.pos=0)}return this.length+=e.length,this.roundClean(),this}digestInto(e){et(this),pt(e,this),this.finished=!0;let{buffer:n,view:r,blockLen:s,isLE:o}=this,{pos:a}=this;n[a++]=128,this.buffer.subarray(a).fill(0),this.padOffset>s-a&&(this.process(r,0),a=0);for(let h=a;h<s;h++)n[h]=0;xn(r,s-8,BigInt(this.length*8),o),this.process(r,0);let c=St(e),i=this.outputLen;if(i%4)throw new Error("_sha2: outputLen should be aligned to 32bit");let u=i/4,f=this.get();if(u>f.length)throw new Error("_sha2: outputLen bigger than state");for(let h=0;h<u;h++)c.setUint32(4*h,f[h],o)}digest(){let{buffer:e,outputLen:n}=this;this.digestInto(e);let r=e.slice(0,n);return this.destroy(),r}_cloneInto(e){e||(e=new this.constructor),e.set(...this.get());let{blockLen:n,buffer:r,length:s,finished:o,destroyed:a,pos:c}=this;return e.length=s,e.pos=c,e.finished=o,e.destroyed=a,s%n&&e.buffer.set(r),e}};var[bn,gn]=y.split(["0x428a2f98d728ae22","0x7137449123ef65cd","0xb5c0fbcfec4d3b2f","0xe9b5dba58189dbbc","0x3956c25bf348b538","0x59f111f1b605d019","0x923f82a4af194f9b","0xab1c5ed5da6d8118","0xd807aa98a3030242","0x12835b0145706fbe","0x243185be4ee4b28c","0x550c7dc3d5ffb4e2","0x72be5d74f27b896f","0x80deb1fe3b1696b1","0x9bdc06a725c71235","0xc19bf174cf692694","0xe49b69c19ef14ad2","0xefbe4786384f25e3","0x0fc19dc68b8cd5b5","0x240ca1cc77ac9c65","0x2de92c6f592b0275","0x4a7484aa6ea6e483","0x5cb0a9dcbd41fbd4","0x76f988da831153b5","0x983e5152ee66dfab","0xa831c66d2db43210","0xb00327c898fb213f","0xbf597fc7beef0ee4","0xc6e00bf33da88fc2","0xd5a79147930aa725","0x06ca6351e003826f","0x142929670a0e6e70","0x27b70a8546d22ffc","0x2e1b21385c26c926","0x4d2c6dfc5ac42aed","0x53380d139d95b3df","0x650a73548baf63de","0x766a0abb3c77b2a8","0x81c2c92e47edaee6","0x92722c851482353b","0xa2bfe8a14cf10364","0xa81a664bbc423001","0xc24b8b70d0f89791","0xc76c51a30654be30","0xd192e819d6ef5218","0xd69906245565a910","0xf40e35855771202a","0x106aa07032bbd1b8","0x19a4c116b8d2d0c8","0x1e376c085141ab53","0x2748774cdf8eeb99","0x34b0bcb5e19b48a8","0x391c0cb3c5c95a63","0x4ed8aa4ae3418acb","0x5b9cca4f7763e373","0x682e6ff3d6b2b8a3","0x748f82ee5defb2fc","0x78a5636f43172f60","0x84c87814a1f0ab72","0x8cc702081a6439ec","0x90befffa23631e28","0xa4506cebde82bde9","0xbef9a3f7b2c67915","0xc67178f2e372532b","0xca273eceea26619c","0xd186b8c721c0c207","0xeada7dd6cde0eb1e","0xf57d4f7fee6ed178","0x06f067aa72176fba","0x0a637dc5a2c898a6","0x113f9804bef90dae","0x1b710b35131c471b","0x28db77f523047d84","0x32caab7b40c72493","0x3c9ebe0a15c9bebc","0x431d67c49c100d4c","0x4cc5d4becb3e42b6","0x597f299cfc657e2a","0x5fcb6fab3ad6faec","0x6c44198c4a475817"].map(t=>BigInt(t))),rt=new Uint32Array(80),st=new Uint32Array(80),Vt=class extends Ot{constructor(){super(128,64,16,!1),this.Ah=1779033703,this.Al=-205731576,this.Bh=-1150833019,this.Bl=-2067093701,this.Ch=1013904242,this.Cl=-23791573,this.Dh=-1521486534,this.Dl=1595750129,this.Eh=1359893119,this.El=-1377402159,this.Fh=-1694144372,this.Fl=725511199,this.Gh=528734635,this.Gl=-79577749,this.Hh=1541459225,this.Hl=327033209}get(){let{Ah:e,Al:n,Bh:r,Bl:s,Ch:o,Cl:a,Dh:c,Dl:i,Eh:u,El:f,Fh:h,Fl:p,Gh:b,Gl:E,Hh:g,Hl:I}=this;return[e,n,r,s,o,a,c,i,u,f,h,p,b,E,g,I]}set(e,n,r,s,o,a,c,i,u,f,h,p,b,E,g,I){this.Ah=e|0,this.Al=n|0,this.Bh=r|0,this.Bl=s|0,this.Ch=o|0,this.Cl=a|0,this.Dh=c|0,this.Dl=i|0,this.Eh=u|0,this.El=f|0,this.Fh=h|0,this.Fl=p|0,this.Gh=b|0,this.Gl=E|0,this.Hh=g|0,this.Hl=I|0}process(e,n){for(let d=0;d<16;d++,n+=4)rt[d]=e.getUint32(n),st[d]=e.getUint32(n+=4);for(let d=16;d<80;d++){let B=rt[d-15]|0,H=st[d-15]|0,Z=y.rotrSH(B,H,1)^y.rotrSH(B,H,8)^y.shrSH(B,H,7),G=y.rotrSL(B,H,1)^y.rotrSL(B,H,8)^y.shrSL(B,H,7),M=rt[d-2]|0,V=st[d-2]|0,lt=y.rotrSH(M,V,19)^y.rotrBH(M,V,61)^y.shrSH(M,V,6),it=y.rotrSL(M,V,19)^y.rotrBL(M,V,61)^y.shrSL(M,V,6),dt=y.add4L(G,it,st[d-7],st[d-16]),yt=y.add4H(dt,Z,lt,rt[d-7],rt[d-16]);rt[d]=yt|0,st[d]=dt|0}let{Ah:r,Al:s,Bh:o,Bl:a,Ch:c,Cl:i,Dh:u,Dl:f,Eh:h,El:p,Fh:b,Fl:E,Gh:g,Gl:I,Hh:S,Hl:k}=this;for(let d=0;d<80;d++){let B=y.rotrSH(h,p,14)^y.rotrSH(h,p,18)^y.rotrBH(h,p,41),H=y.rotrSL(h,p,14)^y.rotrSL(h,p,18)^y.rotrBL(h,p,41),Z=h&b^~h&g,G=p&E^~p&I,M=y.add5L(k,H,G,gn[d],st[d]),V=y.add5H(M,S,B,Z,bn[d],rt[d]),lt=M|0,it=y.rotrSH(r,s,28)^y.rotrBH(r,s,34)^y.rotrBH(r,s,39),dt=y.rotrSL(r,s,28)^y.rotrBL(r,s,34)^y.rotrBL(r,s,39),yt=r&o^r&c^o&c,Ht=s&a^s&i^a&i;S=g|0,k=I|0,g=b|0,I=E|0,b=h|0,E=p|0,{h,l:p}=y.add(u|0,f|0,V|0,lt|0),u=c|0,f=i|0,c=o|0,i=a|0,o=r|0,a=s|0;let Ct=y.add3L(lt,dt,Ht);r=y.add3H(Ct,V,it,yt),s=Ct|0}({h:r,l:s}=y.add(this.Ah|0,this.Al|0,r|0,s|0)),{h:o,l:a}=y.add(this.Bh|0,this.Bl|0,o|0,a|0),{h:c,l:i}=y.add(this.Ch|0,this.Cl|0,c|0,i|0),{h:u,l:f}=y.add(this.Dh|0,this.Dl|0,u|0,f|0),{h,l:p}=y.add(this.Eh|0,this.El|0,h|0,p|0),{h:b,l:E}=y.add(this.Fh|0,this.Fl|0,b|0,E|0),{h:g,l:I}=y.add(this.Gh|0,this.Gl|0,g|0,I|0),{h:S,l:k}=y.add(this.Hh|0,this.Hl|0,S|0,k|0),this.set(r,s,o,a,c,i,u,f,h,p,b,E,g,I,S,k)}roundClean(){rt.fill(0),st.fill(0)}destroy(){this.buffer.fill(0),this.set(0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0)}};var xe=ue(()=>new Vt);var ge=BigInt(0),we=BigInt(1),wn=BigInt(2);function $t(t){return t instanceof Uint8Array||t!=null&&typeof t=="object"&&t.constructor.name==="Uint8Array"}function Zt(t){if(!$t(t))throw new Error("Uint8Array expected")}function kt(t,e){if(typeof e!="boolean")throw new Error(`${t} must be valid boolean, got "${e}".`)}var yn=Array.from({length:256},(t,e)=>e.toString(16).padStart(2,"0"));function Tt(t){Zt(t);let e="";for(let n=0;n<t.length;n++)e+=yn[t[n]];return e}function ye(t){if(typeof t!="string")throw new Error("hex string expected, got "+typeof t);return BigInt(t===""?"0":`0x${t}`)}var J={_0:48,_9:57,_A:65,_F:70,_a:97,_f:102};function be(t){if(t>=J._0&&t<=J._9)return t-J._0;if(t>=J._A&&t<=J._F)return t-(J._A-10);if(t>=J._a&&t<=J._f)return t-(J._a-10)}function me(t){if(typeof t!="string")throw new Error("hex string expected, got "+typeof t);let e=t.length,n=e/2;if(e%2)throw new Error("padded hex string expected, got unpadded hex of length "+e);let r=new Uint8Array(n);for(let s=0,o=0;s<n;s++,o+=2){let a=be(t.charCodeAt(o)),c=be(t.charCodeAt(o+1));if(a===void 0||c===void 0){let i=t[o]+t[o+1];throw new Error('hex string expected, got non-hex character "'+i+'" at index '+o)}r[s]=a*16+c}return r}function Be(t){return ye(Tt(t))}function bt(t){return Zt(t),ye(Tt(Uint8Array.from(t).reverse()))}function zt(t,e){return me(t.toString(16).padStart(e*2,"0"))}function It(t,e){return zt(t,e).reverse()}function K(t,e,n){let r;if(typeof e=="string")try{r=me(e)}catch(o){throw new Error(`${t} must be valid hex string, got "${e}". Cause: ${o}`)}else if($t(e))r=Uint8Array.from(e);else throw new Error(`${t} must be hex string or Uint8Array`);let s=r.length;if(typeof n=="number"&&s!==n)throw new Error(`${t} expected ${n} bytes, got ${s}`);return r}function Gt(...t){let e=0;for(let r=0;r<t.length;r++){let s=t[r];Zt(s),e+=s.length}let n=new Uint8Array(e);for(let r=0,s=0;r<t.length;r++){let o=t[r];n.set(o,s),s+=o.length}return n}var Ft=t=>typeof t=="bigint"&&ge<=t;function mn(t,e,n){return Ft(t)&&Ft(e)&&Ft(n)&&e<=t&&t<n}function gt(t,e,n,r){if(!mn(e,n,r))throw new Error(`expected valid ${t}: ${n} <= n < ${r}, got ${typeof e} ${e}`)}function Ee(t){let e;for(e=0;t>ge;t>>=we,e+=1);return e}var Ie=t=>(wn<<BigInt(t-1))-we;var Bn={bigint:t=>typeof t=="bigint",function:t=>typeof t=="function",boolean:t=>typeof t=="boolean",string:t=>typeof t=="string",stringOrUint8Array:t=>typeof t=="string"||$t(t),isSafeInteger:t=>Number.isSafeInteger(t),array:t=>Array.isArray(t),field:(t,e)=>e.Fp.isValid(t),hash:t=>typeof t=="function"&&Number.isSafeInteger(t.outputLen)};function wt(t,e,n={}){let r=(s,o,a)=>{let c=Bn[o];if(typeof c!="function")throw new Error(`Invalid validator "${o}", expected function`);let i=t[s];if(!(a&&i===void 0)&&!c(i,t))throw new Error(`Invalid param ${String(s)}=${i} (${typeof i}), expected ${o}`)};for(let[s,o]of Object.entries(e))r(s,o,!1);for(let[s,o]of Object.entries(n))r(s,o,!0);return t}function Xt(t){let e=new WeakMap;return(n,...r)=>{let s=e.get(n);if(s!==void 0)return s;let o=t(n,...r);return e.set(n,o),o}}var N=BigInt(0),R=BigInt(1),ht=BigInt(2),In=BigInt(3),jt=BigInt(4),_e=BigInt(5),Ae=BigInt(8),_n=BigInt(9),An=BigInt(16);function C(t,e){let n=t%e;return n>=N?n:e+n}function Sn(t,e,n){if(n<=N||e<N)throw new Error("Expected power/modulo > 0");if(n===R)return N;let r=R;for(;e>N;)e&R&&(r=r*t%n),t=t*t%n,e>>=R;return r}function X(t,e,n){let r=t;for(;e-- >N;)r*=r,r%=n;return r}function Se(t,e){if(t===N||e<=N)throw new Error(`invert: expected positive integers, got n=${t} mod=${e}`);let n=C(t,e),r=e,s=N,o=R,a=R,c=N;for(;n!==N;){let u=r/n,f=r%n,h=s-a*u,p=o-c*u;r=n,n=f,s=a,o=c,a=h,c=p}if(r!==R)throw new Error("invert: does not exist");return C(s,e)}function Ln(t){let e=(t-R)/ht,n,r,s;for(n=t-R,r=0;n%ht===N;n/=ht,r++);for(s=ht;s<t&&Sn(s,e,t)!==t-R;s++);if(r===1){let a=(t+R)/jt;return function(i,u){let f=i.pow(u,a);if(!i.eql(i.sqr(f),u))throw new Error("Cannot find square root");return f}}let o=(n+R)/ht;return function(c,i){if(c.pow(i,e)===c.neg(c.ONE))throw new Error("Cannot find square root");let u=r,f=c.pow(c.mul(c.ONE,s),n),h=c.pow(i,o),p=c.pow(i,n);for(;!c.eql(p,c.ONE);){if(c.eql(p,c.ZERO))return c.ZERO;let b=1;for(let g=c.sqr(p);b<u&&!c.eql(g,c.ONE);b++)g=c.sqr(g);let E=c.pow(f,R<<BigInt(u-b-1));f=c.sqr(E),h=c.mul(h,E),p=c.mul(p,f),u=b}return h}}function On(t){if(t%jt===In){let e=(t+R)/jt;return function(r,s){let o=r.pow(s,e);if(!r.eql(r.sqr(o),s))throw new Error("Cannot find square root");return o}}if(t%Ae===_e){let e=(t-_e)/Ae;return function(r,s){let o=r.mul(s,ht),a=r.pow(o,e),c=r.mul(s,a),i=r.mul(r.mul(c,ht),a),u=r.mul(c,r.sub(i,r.ONE));if(!r.eql(r.sqr(u),s))throw new Error("Cannot find square root");return u}}return t%An,Ln(t)}var Le=(t,e)=>(C(t,e)&R)===R,kn=["create","isValid","is0","neg","inv","sqrt","sqr","eql","add","sub","mul","pow","div","addN","subN","mulN","sqrN"];function Oe(t){let e={ORDER:"bigint",MASK:"bigint",BYTES:"isSafeInteger",BITS:"isSafeInteger"},n=kn.reduce((r,s)=>(r[s]="function",r),e);return wt(t,n)}function Tn(t,e,n){if(n<N)throw new Error("Expected power > 0");if(n===N)return t.ONE;if(n===R)return e;let r=t.ONE,s=e;for(;n>N;)n&R&&(r=t.mul(r,s)),s=t.sqr(s),n>>=R;return r}function Un(t,e){let n=new Array(e.length),r=e.reduce((o,a,c)=>t.is0(a)?o:(n[c]=o,t.mul(o,a)),t.ONE),s=t.inv(r);return e.reduceRight((o,a,c)=>t.is0(a)?o:(n[c]=t.mul(o,n[c]),t.mul(o,a)),s),n}function Pt(t,e){let n=e!==void 0?e:t.toString(2).length,r=Math.ceil(n/8);return{nBitLength:n,nByteLength:r}}function Ut(t,e,n=!1,r={}){if(t<=N)throw new Error(`Expected Field ORDER > 0, got ${t}`);let{nBitLength:s,nByteLength:o}=Pt(t,e);if(o>2048)throw new Error("Field lengths over 2048 bytes are not supported");let a=On(t),c=Object.freeze({ORDER:t,BITS:s,BYTES:o,MASK:Ie(s),ZERO:N,ONE:R,create:i=>C(i,t),isValid:i=>{if(typeof i!="bigint")throw new Error(`Invalid field element: expected bigint, got ${typeof i}`);return N<=i&&i<t},is0:i=>i===N,isOdd:i=>(i&R)===R,neg:i=>C(-i,t),eql:(i,u)=>i===u,sqr:i=>C(i*i,t),add:(i,u)=>C(i+u,t),sub:(i,u)=>C(i-u,t),mul:(i,u)=>C(i*u,t),pow:(i,u)=>Tn(c,i,u),div:(i,u)=>C(i*Se(u,t),t),sqrN:i=>i*i,addN:(i,u)=>i+u,subN:(i,u)=>i-u,mulN:(i,u)=>i*u,inv:i=>Se(i,t),sqrt:r.sqrt||(i=>a(c,i)),invertBatch:i=>Un(c,i),cmov:(i,u,f)=>f?u:i,toBytes:i=>n?It(i,o):zt(i,o),fromBytes:i=>{if(i.length!==o)throw new Error(`Fp.fromBytes: expected ${o}, got ${i.length}`);return n?bt(i):Be(i)}});return Object.freeze(c)}var Rn=BigInt(0),Yt=BigInt(1),Wt=new WeakMap,ke=new WeakMap;function Te(t,e){let n=(o,a)=>{let c=a.negate();return o?c:a},r=o=>{if(!Number.isSafeInteger(o)||o<=0||o>e)throw new Error(`Wrong window size=${o}, should be [1..${e}]`)},s=o=>{r(o);let a=Math.ceil(e/o)+1,c=2**(o-1);return{windows:a,windowSize:c}};return{constTimeNegate:n,unsafeLadder(o,a){let c=t.ZERO,i=o;for(;a>Rn;)a&Yt&&(c=c.add(i)),i=i.double(),a>>=Yt;return c},precomputeWindow(o,a){let{windows:c,windowSize:i}=s(a),u=[],f=o,h=f;for(let p=0;p<c;p++){h=f,u.push(h);for(let b=1;b<i;b++)h=h.add(f),u.push(h);f=h.double()}return u},wNAF(o,a,c){let{windows:i,windowSize:u}=s(o),f=t.ZERO,h=t.BASE,p=BigInt(2**o-1),b=2**o,E=BigInt(o);for(let g=0;g<i;g++){let I=g*u,S=Number(c&p);c>>=E,S>u&&(S-=b,c+=Yt);let k=I,d=I+Math.abs(S)-1,B=g%2!==0,H=S<0;S===0?h=h.add(n(B,a[k])):f=f.add(n(H,a[d]))}return{p:f,f:h}},wNAFCached(o,a,c){let i=ke.get(o)||1,u=Wt.get(o);return u||(u=this.precomputeWindow(o,i),i!==1&&Wt.set(o,c(u))),this.wNAF(i,u,a)},setWindowSize(o,a){r(a),ke.set(o,a),Wt.delete(o)}}}function Ue(t,e,n,r){if(!Array.isArray(n)||!Array.isArray(r)||r.length!==n.length)throw new Error("arrays of points and scalars must have equal length");r.forEach((f,h)=>{if(!e.isValid(f))throw new Error(`wrong scalar at index ${h}`)}),n.forEach((f,h)=>{if(!(f instanceof t))throw new Error(`wrong point at index ${h}`)});let s=Ee(BigInt(n.length)),o=s>12?s-3:s>4?s-2:s?2:1,a=(1<<o)-1,c=new Array(a+1).fill(t.ZERO),i=Math.floor((e.BITS-1)/o)*o,u=t.ZERO;for(let f=i;f>=0;f-=o){c.fill(t.ZERO);for(let p=0;p<r.length;p++){let b=r[p],E=Number(b>>BigInt(f)&BigInt(a));c[E]=c[E].add(n[p])}let h=t.ZERO;for(let p=c.length-1,b=t.ZERO;p>0;p--)b=b.add(c[p]),h=h.add(b);if(u=u.add(h),f!==0)for(let p=0;p<o;p++)u=u.double()}return u}function Re(t){return Oe(t.Fp),wt(t,{n:"bigint",h:"bigint",Gx:"field",Gy:"field"},{nBitLength:"isSafeInteger",nByteLength:"isSafeInteger"}),Object.freeze({...Pt(t.n,t.nBitLength),...t,p:t.Fp.ORDER})}var j=BigInt(0),$=BigInt(1),Rt=BigInt(2),Hn=BigInt(8),Cn={zip215:!0};function Nn(t){let e=Re(t);return wt(t,{hash:"function",a:"bigint",d:"bigint",randomBytes:"function"},{adjustScalarBytes:"function",domain:"function",uvRatio:"function",mapToCurve:"function"}),Object.freeze({...e})}function He(t){let e=Nn(t),{Fp:n,n:r,prehash:s,hash:o,randomBytes:a,nByteLength:c,h:i}=e,u=Rt<<BigInt(c*8)-$,f=n.create,h=Ut(e.n,e.nBitLength),p=e.uvRatio||((w,l)=>{try{return{isValid:!0,value:n.sqrt(w*n.inv(l))}}catch{return{isValid:!1,value:j}}}),b=e.adjustScalarBytes||(w=>w),E=e.domain||((w,l,x)=>{if(kt("phflag",x),l.length||x)throw new Error("Contexts/pre-hash are not supported");return w});function g(w,l){gt("coordinate "+w,l,j,u)}function I(w){if(!(w instanceof d))throw new Error("ExtendedPoint expected")}let S=Xt((w,l)=>{let{ex:x,ey:m,ez:_}=w,A=w.is0();l==null&&(l=A?Hn:n.inv(_));let L=f(x*l),T=f(m*l),O=f(_*l);if(A)return{x:j,y:$};if(O!==$)throw new Error("invZ was invalid");return{x:L,y:T}}),k=Xt(w=>{let{a:l,d:x}=e;if(w.is0())throw new Error("bad point: ZERO");let{ex:m,ey:_,ez:A,et:L}=w,T=f(m*m),O=f(_*_),U=f(A*A),v=f(U*U),F=f(T*l),P=f(U*f(F+O)),Y=f(v+f(x*f(T*O)));if(P!==Y)throw new Error("bad point: equation left != right (1)");let z=f(m*_),tt=f(A*L);if(z!==tt)throw new Error("bad point: equation left != right (2)");return!0});class d{constructor(l,x,m,_){this.ex=l,this.ey=x,this.ez=m,this.et=_,g("x",l),g("y",x),g("z",m),g("t",_),Object.freeze(this)}get x(){return this.toAffine().x}get y(){return this.toAffine().y}static fromAffine(l){if(l instanceof d)throw new Error("extended point not allowed");let{x,y:m}=l||{};return g("x",x),g("y",m),new d(x,m,$,f(x*m))}static normalizeZ(l){let x=n.invertBatch(l.map(m=>m.ez));return l.map((m,_)=>m.toAffine(x[_])).map(d.fromAffine)}static msm(l,x){return Ue(d,h,l,x)}_setWindowSize(l){Z.setWindowSize(this,l)}assertValidity(){k(this)}equals(l){I(l);let{ex:x,ey:m,ez:_}=this,{ex:A,ey:L,ez:T}=l,O=f(x*T),U=f(A*_),v=f(m*T),F=f(L*_);return O===U&&v===F}is0(){return this.equals(d.ZERO)}negate(){return new d(f(-this.ex),this.ey,this.ez,f(-this.et))}double(){let{a:l}=e,{ex:x,ey:m,ez:_}=this,A=f(x*x),L=f(m*m),T=f(Rt*f(_*_)),O=f(l*A),U=x+m,v=f(f(U*U)-A-L),F=O+L,P=F-T,Y=O-L,z=f(v*P),tt=f(F*Y),ct=f(v*Y),_t=f(P*F);return new d(z,tt,_t,ct)}add(l){I(l);let{a:x,d:m}=e,{ex:_,ey:A,ez:L,et:T}=this,{ex:O,ey:U,ez:v,et:F}=l;if(x===BigInt(-1)){let te=f((A-_)*(U+O)),ee=f((A+_)*(U-O)),Nt=f(ee-te);if(Nt===j)return this.double();let ne=f(L*Rt*F),oe=f(T*Rt*v),re=oe+ne,se=ee+te,ie=oe-ne,Fe=f(re*Nt),$e=f(se*ie),Ze=f(re*ie),ze=f(Nt*se);return new d(Fe,$e,ze,Ze)}let P=f(_*O),Y=f(A*U),z=f(T*m*F),tt=f(L*v),ct=f((_+A)*(O+U)-P-Y),_t=tt-z,Qt=tt+z,Jt=f(Y-x*P),qe=f(ct*_t),De=f(Qt*Jt),Me=f(ct*Jt),Ve=f(_t*Qt);return new d(qe,De,Ve,Me)}subtract(l){return this.add(l.negate())}wNAF(l){return Z.wNAFCached(this,l,d.normalizeZ)}multiply(l){let x=l;gt("scalar",x,$,r);let{p:m,f:_}=this.wNAF(x);return d.normalizeZ([m,_])[0]}multiplyUnsafe(l){let x=l;return gt("scalar",x,j,r),x===j?H:this.equals(H)||x===$?this:this.equals(B)?this.wNAF(x).p:Z.unsafeLadder(this,x)}isSmallOrder(){return this.multiplyUnsafe(i).is0()}isTorsionFree(){return Z.unsafeLadder(this,r).is0()}toAffine(l){return S(this,l)}clearCofactor(){let{h:l}=e;return l===$?this:this.multiplyUnsafe(l)}static fromHex(l,x=!1){let{d:m,a:_}=e,A=n.BYTES;l=K("pointHex",l,A),kt("zip215",x);let L=l.slice(),T=l[A-1];L[A-1]=T&-129;let O=bt(L),U=x?u:n.ORDER;gt("pointHex.y",O,j,U);let v=f(O*O),F=f(v-$),P=f(m*v-_),{isValid:Y,value:z}=p(F,P);if(!Y)throw new Error("Point.fromHex: invalid y coordinate");let tt=(z&$)===$,ct=(T&128)!==0;if(!x&&z===j&&ct)throw new Error("Point.fromHex: x=0 and x_0=1");return ct!==tt&&(z=f(-z)),d.fromAffine({x:z,y:O})}static fromPrivateKey(l){return V(l).point}toRawBytes(){let{x:l,y:x}=this.toAffine(),m=It(x,n.BYTES);return m[m.length-1]|=l&$?128:0,m}toHex(){return Tt(this.toRawBytes())}}d.BASE=new d(e.Gx,e.Gy,$,f(e.Gx*e.Gy)),d.ZERO=new d(j,$,$,j);let{BASE:B,ZERO:H}=d,Z=Te(d,c*8);function G(w){return C(w,r)}function M(w){return G(bt(w))}function V(w){let l=c;w=K("private key",w,l);let x=K("hashed private key",o(w),2*l),m=b(x.slice(0,l)),_=x.slice(l,2*l),A=M(m),L=B.multiply(A),T=L.toRawBytes();return{head:m,prefix:_,scalar:A,point:L,pointBytes:T}}function lt(w){return V(w).pointBytes}function it(w=new Uint8Array,...l){let x=Gt(...l);return M(o(E(x,K("context",w),!!s)))}function dt(w,l,x={}){w=K("message",w),s&&(w=s(w));let{prefix:m,scalar:_,pointBytes:A}=V(l),L=it(x.context,m,w),T=B.multiply(L).toRawBytes(),O=it(x.context,T,A,w),U=G(L+O*_);gt("signature.s",U,j,r);let v=Gt(T,It(U,n.BYTES));return K("result",v,c*2)}let yt=Cn;function Ht(w,l,x,m=yt){let{context:_,zip215:A}=m,L=n.BYTES;w=K("signature",w,2*L),l=K("message",l),A!==void 0&&kt("zip215",A),s&&(l=s(l));let T=bt(w.slice(L,2*L)),O,U,v;try{O=d.fromHex(x,A),U=d.fromHex(w.slice(0,L),A),v=B.multiplyUnsafe(T)}catch{return!1}if(!A&&O.isSmallOrder())return!1;let F=it(_,U.toRawBytes(),O.toRawBytes(),l);return U.add(O.multiplyUnsafe(F)).subtract(v).clearCofactor().equals(d.ZERO)}return B._setWindowSize(8),{CURVE:e,getPublicKey:lt,sign:dt,verify:Ht,ExtendedPoint:d,utils:{getExtendedPublicKey:V,randomPrivateKey:()=>a(n.BYTES),precompute(w=8,l=d.BASE){return l._setWindowSize(w),l.multiply(BigInt(3)),l}}}}var Kt=BigInt("57896044618658097711785492504343953926634992332820282019728792003956564819949"),Ce=BigInt("19681161376707505956807079304988542015446066515923890162744021073123829784752"),Ro=BigInt(0),vn=BigInt(1),Ne=BigInt(2),Ho=BigInt(3),qn=BigInt(5),Dn=BigInt(8);function Mn(t){let e=BigInt(10),n=BigInt(20),r=BigInt(40),s=BigInt(80),o=Kt,c=t*t%o*t%o,i=X(c,Ne,o)*c%o,u=X(i,vn,o)*t%o,f=X(u,qn,o)*u%o,h=X(f,e,o)*f%o,p=X(h,n,o)*h%o,b=X(p,r,o)*p%o,E=X(b,s,o)*b%o,g=X(E,s,o)*b%o,I=X(g,e,o)*f%o;return{pow_p_5_8:X(I,Ne,o)*t%o,b2:c}}function Vn(t){return t[0]&=248,t[31]&=127,t[31]|=64,t}function Fn(t,e){let n=Kt,r=C(e*e*e,n),s=C(r*r*e,n),o=Mn(t*s).pow_p_5_8,a=C(t*r*o,n),c=C(e*a*a,n),i=a,u=C(a*Ce,n),f=c===t,h=c===C(-t,n),p=c===C(-t*Ce,n);return f&&(a=i),(h||p)&&(a=u),Le(a,n)&&(a=C(-a,n)),{isValid:f||h,value:a}}var $n=Ut(Kt,void 0,!0),Zn={a:BigInt(-1),d:BigInt("37095705934669439343138083508754565189542113879843219016388785533085940283555"),Fp:$n,n:BigInt("7237005577332262213973186563042994240857116359379907606001950938285454250989"),h:Dn,Gx:BigInt("15112221349535400772501151409588531511454012693041857206046113283949847762202"),Gy:BigInt("46316835694926478169428394003475163141307993866256225615783033603165251855960"),hash:xe,randomBytes:le,adjustScalarBytes:Vn,uvRatio:Fn},ve=He(Zn);globalThis.ememCrypto={blake3:pe,ed25519:ve};})();
/*! Bundled license information:

@noble/hashes/esm/utils.js:
  (*! noble-hashes - MIT License (c) 2022 Paul Miller (paulmillr.com) *)

@noble/curves/esm/abstract/utils.js:
@noble/curves/esm/abstract/modular.js:
@noble/curves/esm/abstract/curve.js:
@noble/curves/esm/abstract/edwards.js:
@noble/curves/esm/ed25519.js:
  (*! noble-curves - MIT License (c) 2022 Paul Miller (paulmillr.com) *)
*/


// Bound from the bundle above rather than imported, so this file has no
// module graph and can be inlined verbatim into an HTML document.
var nobleBlake3 = globalThis.ememCrypto.blake3;
var nobleEd     = globalThis.ememCrypto.ed25519;

const B32A = 'abcdefghijklmnopqrstuvwxyz234567';
function b32decode(s){
  s = s.toLowerCase().replace(/=+$/, '');
  const out = new Uint8Array(Math.floor(s.length * 5 / 8));
  let bits = 0, value = 0, idx = 0;
  for (const c of s) {
    const i = B32A.indexOf(c);
    if (i < 0) throw new Error('bad base32 char: ' + c);
    value = (value << 5) | i;
    bits += 5;
    if (bits >= 8) {
      out[idx++] = (value >>> (bits - 8)) & 0xff;
      bits -= 8;
    }
  }
  return out.slice(0, idx);
}

// ── Minimal canonical-CBOR encoder ──────────────────────────────────────────
// Reproduces ciborium's output for the four shapes the preimage needs:
// definite-length text strings, unsigned ints (shortest form), a
// definite array of text, and a definite map of string→string in a given
// key order. Validated at load time against vectors emitted by the Rust
// signer (see CBOR_VECTORS / selfTestCbor); if a vector mismatches, the
// browser path disables itself and the page verifies via the server.
const _enc = new TextEncoder();
function cborHead(major, n, out){
  const mt = major << 5;
  if (n < 24) { out.push(mt | n); }
  else if (n < 0x100) { out.push(mt | 24, n & 0xff); }
  else if (n < 0x10000) { out.push(mt | 25, (n>>>8)&0xff, n&0xff); }
  else if (n < 0x100000000) { out.push(mt | 26, (n>>>24)&0xff,(n>>>16)&0xff,(n>>>8)&0xff,n&0xff); }
  else {
    // 64-bit: split via BigInt for values beyond 2^32 (e.g. far-future tslot).
    const b = BigInt(n);
    out.push(mt | 27);
    for (let s = 56n; s >= 0n; s -= 8n) out.push(Number((b >> s) & 0xffn));
  }
}
function cborText(str, out){
  const b = _enc.encode(str);
  cborHead(3, b.length, out);
  for (let i=0;i<b.length;i++) out.push(b[i]);
}
function cborUint(n, out){ cborHead(0, n, out); }
function cborArrayText(arr, out){
  cborHead(4, arr.length, out);
  for (const s of arr) cborText(s, out);
}
// Map with keys emitted in the EXACT order supplied (caller pre-sorts to
// match the server's BTreeMap iteration order).
function cborMapStr(pairs, out){
  cborHead(5, pairs.length, out);
  for (const [k,v] of pairs){ cborText(k, out); cborText(v, out); }
}
function u8(arr){ return new Uint8Array(arr); }
function hex(u8a){ let s=''; for (let i=0;i<u8a.length;i++) s+=u8a[i].toString(16).padStart(2,'0'); return s; }

// blake3 → lowercase hex of canonical CBOR of source_versions (BTreeMap,
// key-sorted). Returns null for an empty map so the segment is omitted.
function manifestHex(receipt){
  const sv = receipt.source_versions || {};
  const keys = Object.keys(sv).sort();
  if (keys.length === 0) return null;
  const out = [];
  cborMapStr(keys.map(k => [k, String(sv[k])]), out);
  return hex(nobleBlake3(u8(out)));
}
// blake3 hex of canonical CBOR of the SORTED edge-cid string list. null when empty.
function edgesHex(receipt){
  const eds = (receipt.edge_cids || []).map(String).slice().sort();
  if (eds.length === 0) return null;
  const out = []; cborArrayText(eds, out);
  return hex(nobleBlake3(u8(out)));
}
// blake3 hex of canonical CBOR of a non-empty Scope (declaration order:
// user_id, agent_id, run_id, org_id; absent fields omitted). null when empty.
function scopeHex(receipt){
  const sc = receipt.scope; if (!sc) return null;
  const pairs = [];
  for (const k of ['user_id','agent_id','run_id','org_id'])
    if (sc[k] != null) pairs.push([k, String(sc[k])]);
  if (pairs.length === 0) return null;
  const out = []; cborMapStr(pairs, out);
  return hex(nobleBlake3(u8(out)));
}
// blake3 hex of canonical CBOR of a bounded AsOfReceipt (valid_time:u64,
// transaction_time:string; absent fields omitted). null when unbounded.
function asOfHex(receipt){
  const a = receipt.as_of; if (!a) return null;
  const pairs = [];
  const out = [];
  let n = 0;
  if (a.valid_time != null) n++;
  if (a.transaction_time != null) n++;
  if (n === 0) return null;
  cborHead(5, n, out);
  if (a.valid_time != null){ cborText('valid_time', out); cborUint(a.valid_time, out); }
  if (a.transaction_time != null){ cborText('transaction_time', out); cborText(String(a.transaction_time), out); }
  return hex(nobleBlake3(u8(out)));
}

// ── v1 preimage (preimage_version >= 1) ─────────────────────────────────────
// PreimageV1: blake3( "emem.preimage.v1\x00" || u32le(len(domain)) || domain
//   || seg* ), where each fixed segment is  tag(1) || u32le(len) || bytes  and
//   each list segment is  tag(1) || u32le(count) || (u32le(len)||bytes)* .
// Mirrors emem-attest::receipt_preimage_v1 byte-for-byte.
const RT = { REQUEST_ID:1, SERVED_AT:2, SCOPE:3, AS_OF:4, EDGES:5, MANIFEST:6, PRIMITIVE:7, CELLS:8, FACT_CIDS:9,
             FIELD:10, MERKLE:11 };
// Sub-preimage tags for the v2 merkle binding (emem-attest::merkle_tag).
const MT = { ROOT:1, LEAF_INDEX:2, PATH:3, RULE_VERSION:4, ABSENT:5 };
function u32le(n, out){ out.push(n&0xff,(n>>>8)&0xff,(n>>>16)&0xff,(n>>>24)&0xff); }
function v1Seg(tag, str, out){
  const b = _enc.encode(str); out.push(tag); u32le(b.length, out);
  for (let i=0;i<b.length;i++) out.push(b[i]);
}
function v1SegList(tag, arr, out){
  out.push(tag); u32le(arr.length, out);
  for (const s of arr){ const b=_enc.encode(s); u32le(b.length,out); for (let i=0;i<b.length;i++) out.push(b[i]); }
}
// Raw-byte segment (the merkle sub-preimage binds hashes, not strings).
function v1SegBytes(tag, bytes, out){
  out.push(tag); u32le(bytes.length, out);
  for (let i=0;i<bytes.length;i++) out.push(bytes[i]);
}

// Accept the three shapes a 32-byte hash arrives in over the wire: a JSON
// array of octets (serde's default for [u8;32]), lowercase hex, or base32.
function _bytesOf(v){
  if (Array.isArray(v)) return new Uint8Array(v);
  if (typeof v === 'string') {
    if (/^[0-9a-fA-F]+$/.test(v) && v.length % 2 === 0) {
      const o = new Uint8Array(v.length/2);
      for (let i=0;i<o.length;i++) o[i] = parseInt(v.substr(i*2,2),16);
      return o;
    }
    try { return b32decode(v); } catch(_) {}
  }
  return new Uint8Array(0);
}

// blake3 hex of the field binding — emem-attest::field_binding_v1. A tagged
// sub-preimage over (aoi_cid, derivation_cid), not CBOR like scope/as_of.
// Absent on ordinary receipts; present on field responses.
function fieldHex(receipt){
  const f = receipt.field; if (!f) return null;
  const out = [];
  const dom = _enc.encode('emem.preimage.v1\x00');
  for (let i=0;i<dom.length;i++) out.push(dom[i]);
  const d = _enc.encode('field'); u32le(d.length, out); for (let i=0;i<d.length;i++) out.push(d[i]);
  v1Seg(1, f.aoi_cid || '', out);
  v1Seg(2, f.derivation_cid || '', out);
  return hex(nobleBlake3(u8(out)));
}

// Mirrors emem-attest::merkle_binding_v2. Returns the lowercase-hex digest
// that the receipt's MERKLE segment carries.
//
// A receipt with no proof hashes an explicit ABSENT marker rather than
// skipping the segment. That asymmetry is the entire mechanism: if absence
// were encoded by omission, a proof deleted in transit would hash the same
// as a receipt that never had one, and stripping would stay invisible —
// which is the v1 behaviour this replaces.
function merkleBindingV2Hex(receipt){
  const out = [];
  const dom = _enc.encode('emem.preimage.v1\x00');
  for (let i=0;i<dom.length;i++) out.push(dom[i]);
  const d = _enc.encode('merkle'); u32le(d.length, out); for (let i=0;i<d.length;i++) out.push(d[i]);
  const p = receipt.merkle_proof;
  if (!p) {
    v1SegBytes(MT.ABSENT, [], out);
  } else {
    v1SegBytes(MT.ROOT, _bytesOf(p.root), out);
    const li = []; u32le(Number(p.leaf_index) || 0, li);
    v1SegBytes(MT.LEAF_INDEX, li, out);
    const path = [];
    for (const h of (p.path || [])) { const b = _bytesOf(h); for (let i=0;i<b.length;i++) path.push(b[i]); }
    v1SegBytes(MT.PATH, path, out);
    v1SegBytes(MT.RULE_VERSION, [Number(p.version) || 0], out);
  }
  return hex(nobleBlake3(u8(out)));
}

function buildPreimageV1Bytes(receipt){
  const out = [];
  const dom = _enc.encode('emem.preimage.v1\x00'); // 17 bytes incl NUL
  for (let i=0;i<dom.length;i++) out.push(dom[i]);
  const d = _enc.encode('receipt'); u32le(d.length, out); for (let i=0;i<d.length;i++) out.push(d[i]);
  v1Seg(RT.REQUEST_ID, receipt.request_id || '', out);
  v1Seg(RT.SERVED_AT,  receipt.served_at  || '', out);
  const sh = scopeHex(receipt);    if (sh)  v1Seg(RT.SCOPE, sh, out);
  const ah = asOfHex(receipt);     if (ah)  v1Seg(RT.AS_OF, ah, out);
  const eh = edgesHex(receipt);    if (eh)  v1Seg(RT.EDGES, eh, out);
  const mh = manifestHex(receipt); if (mh)  v1Seg(RT.MANIFEST, mh, out);
  v1Seg(RT.PRIMITIVE, receipt.primitive || '', out);
  v1SegList(RT.CELLS, receipt.cells || [], out);
  v1SegList(RT.FACT_CIDS, receipt.fact_cids || [], out);
  // FIELD then MERKLE, in that order — both are appended after FACT_CIDS
  // so a receipt lacking them hashes identically to one signed before the
  // tags existed. Order is load-bearing, not cosmetic.
  const fh = fieldHex(receipt);    if (fh)  v1Seg(RT.FIELD, fh, out);
  if (Number(receipt.preimage_version) >= 2) {
    v1Seg(RT.MERKLE, merkleBindingV2Hex(receipt), out);
  }
  return u8(out);
}

// ── v0 legacy preimage (preimage_version absent/0) ──────────────────────────
// request_id | served_at | [scope|][as_of|][edges|][manifest|] primitive |
//   (cell ',')* | (cid ',')*  — '|'-joined, each list element trailed by ','.
function buildPreimageV0Bytes(receipt){
  const parts = [];
  const push = s => parts.push(_enc.encode(s));
  push(receipt.request_id || ''); push('|');
  push(receipt.served_at  || ''); push('|');
  const sh = scopeHex(receipt);    if (sh) { push(sh); push('|'); }
  const ah = asOfHex(receipt);     if (ah) { push(ah); push('|'); }
  const eh = edgesHex(receipt);    if (eh) { push(eh); push('|'); }
  const mh = manifestHex(receipt); if (mh) { push(mh); push('|'); }
  push(receipt.primitive || ''); push('|');
  for (const c of (receipt.cells || []))     { push(c); push(','); }
  push('|');
  for (const c of (receipt.fact_cids || [])) { push(c); push(','); }
  let len = 0; for (const p of parts) len += p.length;
  const out = new Uint8Array(len);
  let o = 0; for (const p of parts) { out.set(p, o); o += p.length; }
  return out;
}

// Version dispatch — what the responder actually signed.
function buildPreimageBytes(receipt){
  return (Number(receipt.preimage_version) >= 1)
    ? buildPreimageV1Bytes(receipt)
    : buildPreimageV0Bytes(receipt);
}

// A readable rendering of the preimage for the receipt card. v1 is a
// binary, tagged, length-prefixed stream (not a printable string), so we
// show its labelled segment list; v0 shows the literal '|'-joined string.
function buildPreimageDisplay(receipt){
  if (Number(receipt.preimage_version) >= 1){
    const segs = [];
    segs.push(`domain="receipt"`);
    segs.push(`request_id=${receipt.request_id || ''}`);
    segs.push(`served_at=${receipt.served_at || ''}`);
    const sh = scopeHex(receipt);    if (sh) segs.push(`scope=${sh}`);
    const ah = asOfHex(receipt);     if (ah) segs.push(`as_of=${ah}`);
    const eh = edgesHex(receipt);    if (eh) segs.push(`edges=${eh}`);
    const mh = manifestHex(receipt); if (mh) segs.push(`manifest=${mh}`);
    segs.push(`primitive=${receipt.primitive || ''}`);
    segs.push(`cells=[${(receipt.cells || []).join(', ')}]`);
    segs.push(`fact_cids=[${(receipt.fact_cids || []).join(', ')}]`);
    return 'v1 · blake3(domain-separated, length-prefixed segments):\n  ' + segs.join('\n  ');
  }
  const scopeSeg    = (() => { const h = scopeHex(receipt);    return h ? h + '|' : ''; })();
  const asOfSeg     = (() => { const h = asOfHex(receipt);     return h ? h + '|' : ''; })();
  const edgesSeg    = (() => { const h = edgesHex(receipt);    return h ? h + '|' : ''; })();
  const manifestSeg = (() => { const h = manifestHex(receipt); return h ? h + '|' : ''; })();
  const cells = (receipt.cells || []).join(',') + ',';
  const cids  = (receipt.fact_cids || []).join(',') + ',';
  return `${receipt.request_id || ''}|${receipt.served_at || ''}|${scopeSeg}${asOfSeg}${edgesSeg}${manifestSeg}${receipt.primitive || ''}|${cells}|${cids}`;
}

// Receipts come over the wire with the signature as a 64-element byte
// array (`signature`) and the pubkey as either a 32-byte array
// (`responder`) or the base32 string (`responder_pubkey_b32`). Older /
// CBOR-canonical encodings may also carry `sig_b32` / `signature_b32`.
// Accept all four shapes so a verifier never silently rejects a valid
// receipt because the wire format normalised differently than the
// caller expected.
function _sigBytes(receipt){
  if (Array.isArray(receipt.signature)) return new Uint8Array(receipt.signature);
  if (typeof receipt.signature === 'string') {
    try { return b32decode(receipt.signature); } catch(_) {}
  }
  if (receipt.sig_b32)       return b32decode(receipt.sig_b32);
  if (receipt.signature_b32) return b32decode(receipt.signature_b32);
  return null;
}
function _pubBytes(receipt){
  if (Array.isArray(receipt.responder)) return new Uint8Array(receipt.responder);
  if (receipt.responder_pubkey_b32) return b32decode(receipt.responder_pubkey_b32);
  if (typeof receipt.responder === 'string') {
    try { return b32decode(receipt.responder); } catch(_) {}
  }
  return null;
}
const CBOR_VECTORS = {
  // blake3(canonical_cbor(BTreeMap{bands_cid:bbb,registry_cid:rrr,schema_cid:sss,sources_cid:ooo}))
  manifestHex: 'de14467c03ed214d08ad536ba2923fed93dfd1d2af63c74d1b08131b00ea3915',
  // blake3(canonical_cbor(["e1","e2"]))
  edgesHex: '5a27c406ddc3b90b10c07edd1c0902a4901ede4c0efc48d6d792c92f85d8cca6',
  // blake3(canonical_cbor(Scope{user_id:u1,run_id:r9}))
  scopeHex: '6306e69a1f2e0df252a01f76785440c4f3d0704b3872a01bda85446256e8d45c',
  // blake3(canonical_cbor(AsOfReceipt{valid_time:1767225600,transaction_time:"2026-05-29T00:00:00Z"}))
  asOfHex: '89593e75f23de38f21e520b65c8b86a032cd83f48335bbf95cb7b2b7debbc994',
  // receipt_preimage_v1("RID","2026-06-12T00:00:00Z",Some(aa),None,Some(bb),Some(cc),"emem.recall",[cellA,cellB],[fc1])
  preimageV1: '96562b90bf1a74063fff3807a56f98a162b188bd7ac08fea74147606bcbb67b1',
};
function buildPreimageV1WithHex(reqId, servedAt, sh, ah, eh, mh, primitive, cells, cids){
  const out = [];
  const dom = _enc.encode('emem.preimage.v1\x00');
  for (let i=0;i<dom.length;i++) out.push(dom[i]);
  const d = _enc.encode('receipt'); u32le(d.length, out); for (let i=0;i<d.length;i++) out.push(d[i]);
  v1Seg(RT.REQUEST_ID, reqId, out);
  v1Seg(RT.SERVED_AT, servedAt, out);
  if (sh) v1Seg(RT.SCOPE, sh, out);
  if (ah) v1Seg(RT.AS_OF, ah, out);
  if (eh) v1Seg(RT.EDGES, eh, out);
  if (mh) v1Seg(RT.MANIFEST, mh, out);
  v1Seg(RT.PRIMITIVE, primitive, out);
  v1SegList(RT.CELLS, cells, out);
  v1SegList(RT.FACT_CIDS, cids, out);
  return u8(out);
}
function selfTestEncoders(){
  try {
    const m = manifestHex({ source_versions: { bands_cid:'bbb', registry_cid:'rrr', schema_cid:'sss', sources_cid:'ooo' } });
    if (m !== CBOR_VECTORS.manifestHex) return false;
    const e = edgesHex({ edge_cids: ['e2','e1'] });
    if (e !== CBOR_VECTORS.edgesHex) return false;
    const s = scopeHex({ scope: { user_id:'u1', run_id:'r9' } });
    if (s !== CBOR_VECTORS.scopeHex) return false;
    const a = asOfHex({ as_of: { valid_time:1767225600, transaction_time:'2026-05-29T00:00:00Z' } });
    if (a !== CBOR_VECTORS.asOfHex) return false;
    // The preimage vector uses literal segment hex (aa/bb/cc), not derived
    // digests, so rebuild it with the segment hex injected directly. This
    // exercises the tagging + length-prefix layout end-to-end.
    const preStub = buildPreimageV1WithHex('RID','2026-06-12T00:00:00Z','aa',null,'bb','cc','emem.recall',['cellA','cellB'],['fc1']);
    if (hex(nobleBlake3(preStub)) !== CBOR_VECTORS.preimageV1) return false;
    return true;
  } catch (e) {
    console.warn('[verify] encoder self-test threw:', e);
    return false;
  }
}
// ── the one entry point ─────────────────────────────────────────────────────
// Returns a verdict object and never throws. `ok` is true ONLY when this
// code recomputed the digest and ed25519 accepted the signature over it.
// Every other outcome names itself, so a caller can never render a tick for
// a state where no cryptography ran.
function ememVerifyReceipt(receipt) {
  try {
    if (!selfTestEncoders()) {
      return { ok: false, state: "self_test_failed",
               why: "the encoders in this build disagree with the vectors the Rust signer emitted, so any digest computed here would be wrong. Nothing was checked." };
    }
    if (!receipt || typeof receipt !== "object") {
      return { ok: false, state: "no_receipt", why: "no receipt to check" };
    }
    var pre = buildPreimageBytes(receipt);
    var digest = nobleBlake3(pre);
    var sig = _sigBytes(receipt);
    var pub = _pubBytes(receipt);
    if (!sig || !pub) {
      return { ok: false, state: "incomplete",
               why: "the receipt carries no signature or no responder key" };
    }
    var ok = nobleEd.verify(sig, digest, pub);
    return {
      ok: !!ok,
      state: ok ? "verified" : "signature_rejected",
      digest_hex: hex(digest),
      preimage_version: Number(receipt.preimage_version) || 0,
      signer_b32: receipt.responder_pubkey_b32 || null,
      why: ok
        ? "this code rebuilt the canonical preimage, hashed it with blake3, and ed25519 accepted the signature against the responder key in the receipt"
        : "the signature does not match the digest this code computed from the receipt's own fields",
    };
  } catch (e) {
    return { ok: false, state: "error", why: String(e && e.message || e) };
  }
}

root.ememVerify = {
  verifyReceipt: ememVerifyReceipt,
  selfTest: selfTestEncoders,
  blake3Hex: function (bytes) { return hex(nobleBlake3(bytes)); },
};

// The pieces emem.dev/verify draws its explanation from. That page does not
// only report a verdict, it shows the preimage segment by segment, so it
// needs the encoders themselves rather than the one-call entry point above.
//
// Exported so that page can DELETE its own copy. Two transcriptions of one
// preimage rule is two things to keep in step with emem-attest, and the drift
// would be silent: the second copy would compute a wrong digest and report a
// sound receipt as forged.
root.ememVerifyInternals = {
  ed: nobleEd,
  blake3: nobleBlake3,
  b32decode: b32decode,
  hex: hex,
  u8: u8,
  RT: RT,
  MT: MT,
  manifestHex: manifestHex,
  edgesHex: edgesHex,
  scopeHex: scopeHex,
  asOfHex: asOfHex,
  fieldHex: fieldHex,
  merkleBindingV2Hex: merkleBindingV2Hex,
  buildPreimageV1Bytes: buildPreimageV1Bytes,
  buildPreimageV0Bytes: buildPreimageV0Bytes,
  buildPreimageBytes: buildPreimageBytes,
  buildPreimageDisplay: buildPreimageDisplay,
  sigBytes: _sigBytes,
  pubBytes: _pubBytes,
};
})(typeof globalThis !== "undefined" ? globalThis : this);
