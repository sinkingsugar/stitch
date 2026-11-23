var Ml="187";var Sl=0,ra=1,bl=2;var is=1,wl=2,Oi=3,Bi=0,Qe=1,rn=2,ai=0,oi=1,ss=2,aa=3,oa=4,Al=5;var zi=100,Tl=101,El=102,Rl=103,Cl=104,Il=200,Pl=201,Ll=202,Nl=203,Dl=204,Ul=205,Fl=206,Ol=207,Bl=208,zl=209,kl=210,Gl=211,Hl=212,Wl=213,Vl=214,Xl=0,ql=1,Yl=2,la=3,Zl=4,Jl=5,$l=6,Kl=7,Ql=0,jl=1,tc=2,Sn=0,ca=1,ha=2,ua=3,da=4,fa=5,pa=6,ma=7;var rs=301,ki=302,js=303,tr=304,ec=1000,er=1001,nc=1002,qn=1003,ic=1004;var as=1005;var ln=1006,nr=1007;var Yn=1008;var xn=1009,sc=1010,rc=1011,os=1012,ga=1013,Zn=1014,Un=1015,bn=1016,_a=1017,xa=1018,li=1020,ac=35902,oc=35899,lc=1021,cc=1022,Fn=1023,Gi=1026,Jn=1027,hc=1028,va=1029,ci=1030,ya=1031;var Ma=1033,ir=33776,sr=33777,rr=33778,ar=33779,Sa=35840,ba=35841,wa=35842,Aa=35843,Ta=36196,Ea=37492,Ra=37496,Ca=37488,Ia=37489,or=37490,Pa=37491,La=37808,Na=37809,Da=37810,Ua=37811,Fa=37812,Oa=37813,Ba=37814,za=37815,ka=37816,Ga=37817,Ha=37818,Wa=37819,Va=37820,Xa=37821,qa=36492,Ya=36494,Za=36495,Ja=36283,$a=36284,lr=36285,Ka=36286;var Qa=0,uc=1,hi="",dc="srgb",ja="srgb-linear",to="linear",ye="srgb";var fc=512,pc=513,mc=514,eo=515,gc=516,_c=517,no=518,io=519;var so="300 es",ro=2000;function Ah(t){for(let e=t.length-1;e>=0;--e)if(t[e]>=65535)return!0;return!1}function Th(t){return ArrayBuffer.isView(t)&&!(t instanceof DataView)}function ns(t){return document.createElementNS("http://www.w3.org/1999/xhtml",t)}function xc(){let t=ns("canvas");return t.style.display="block",t}var Zo={},Ui=null;function ao(...t){let e="THREE."+t.shift();if(Ui)Ui("log",e,...t);else console.log(e,...t)}function vc(t){let e=t[0];if(typeof e==="string"&&e.startsWith("TSL:")){let n=t[1];if(n&&n.isStackTrace)t[0]+=" "+n.getLocation();else t[1]='Stack trace not available. Enable "THREE.Node.captureStackTrace" to capture stack traces.'}return t}function Wt(...t){t=vc(t);let e="THREE."+t.shift();if(Ui)Ui("warn",e,...t);else{let n=t[0];if(n&&n.isStackTrace)console.warn(n.getError(e));else console.warn(e,...t)}}function Yt(...t){t=vc(t);let e="THREE."+t.shift();if(Ui)Ui("error",e,...t);else{let n=t[0];if(n&&n.isStackTrace)console.error(n.getError(e));else console.error(e,...t)}}function es(...t){let e=t.join(" ");if(e in Zo)return;Zo[e]=!0,Wt(...t)}function yc(t,e,n){return new Promise(function(i,s){function r(){switch(t.clientWaitSync(e,t.SYNC_FLUSH_COMMANDS_BIT,0)){case t.WAIT_FAILED:s();break;case t.TIMEOUT_EXPIRED:setTimeout(r,n);break;default:i()}}setTimeout(r,n)})}var Mc={[0]:0,[2]:6,[4]:4,[3]:5,[1]:1,[6]:2,[7]:7,[5]:3};class On{addEventListener(t,e){if(this._listeners===void 0)this._listeners={};let n=this._listeners;if(n[t]===void 0)n[t]=[];if(n[t].indexOf(e)===-1)n[t].push(e)}hasEventListener(t,e){let n=this._listeners;if(n===void 0)return!1;return n[t]!==void 0&&n[t].indexOf(e)!==-1}removeEventListener(t,e){let n=this._listeners;if(n===void 0)return;let i=n[t];if(i!==void 0){let s=i.indexOf(e);if(s!==-1)i.splice(s,1)}}dispatchEvent(t){let e=this._listeners;if(e===void 0)return;let n=e[t.type];if(n!==void 0){t.target=this;let i=n.slice(0);for(let s=0,r=i.length;s<r;s++)i[s].call(this,t);t.target=null}}}var He=["00","01","02","03","04","05","06","07","08","09","0a","0b","0c","0d","0e","0f","10","11","12","13","14","15","16","17","18","19","1a","1b","1c","1d","1e","1f","20","21","22","23","24","25","26","27","28","29","2a","2b","2c","2d","2e","2f","30","31","32","33","34","35","36","37","38","39","3a","3b","3c","3d","3e","3f","40","41","42","43","44","45","46","47","48","49","4a","4b","4c","4d","4e","4f","50","51","52","53","54","55","56","57","58","59","5a","5b","5c","5d","5e","5f","60","61","62","63","64","65","66","67","68","69","6a","6b","6c","6d","6e","6f","70","71","72","73","74","75","76","77","78","79","7a","7b","7c","7d","7e","7f","80","81","82","83","84","85","86","87","88","89","8a","8b","8c","8d","8e","8f","90","91","92","93","94","95","96","97","98","99","9a","9b","9c","9d","9e","9f","a0","a1","a2","a3","a4","a5","a6","a7","a8","a9","aa","ab","ac","ad","ae","af","b0","b1","b2","b3","b4","b5","b6","b7","b8","b9","ba","bb","bc","bd","be","bf","c0","c1","c2","c3","c4","c5","c6","c7","c8","c9","ca","cb","cc","cd","ce","cf","d0","d1","d2","d3","d4","d5","d6","d7","d8","d9","da","db","dc","dd","de","df","e0","e1","e2","e3","e4","e5","e6","e7","e8","e9","ea","eb","ec","ed","ee","ef","f0","f1","f2","f3","f4","f5","f6","f7","f8","f9","fa","fb","fc","fd","fe","ff"];var Fr=Math.PI/180,$s=180/Math.PI;function ls(){let t=Math.random()*4294967295|0,e=Math.random()*4294967295|0,n=Math.random()*4294967295|0,i=Math.random()*4294967295|0;return(He[t&255]+He[t>>8&255]+He[t>>16&255]+He[t>>24&255]+"-"+He[e&255]+He[e>>8&255]+"-"+He[e>>16&15|64]+He[e>>24&255]+"-"+He[n&63|128]+He[n>>8&255]+"-"+He[n>>16&255]+He[n>>24&255]+He[i&255]+He[i>>8&255]+He[i>>16&255]+He[i>>24&255]).toLowerCase()}function oe(t,e,n){return Math.max(e,Math.min(n,t))}function Eh(t,e){return(t%e+e)%e}function Or(t,e,n){return(1-n)*t+n*e}function Sc(t){return Math.pow(2,Math.floor(Math.log(t)/Math.LN2))}function oo(t){switch(t.constructor){case Uint32Array:return 4294967295;case Uint16Array:return 65535;case Uint8Array:case Uint8ClampedArray:return 255;case Int32Array:return 2147483647;case Int16Array:return 32767;case Int8Array:return 127;default:return}}function Rh(t,e){if(e.constructor===Float32Array)return t;let n=oo(e);if(n===void 0)throw Error("THREE.MathUtils: Invalid component type.");return Math.max(t/n,-1)}function Ch(t,e){if(e.constructor===Float32Array)return t;let n=oo(e);if(n===void 0)throw Error("THREE.MathUtils: Invalid component type.");return Math.round(t*n)}class fe{static{this.prototype.isVector2=!0}constructor(t=0,e=0){this.x=t,this.y=e}get width(){return this.x}set width(t){this.x=t}get height(){return this.y}set height(t){this.y=t}set(t,e){return this.x=t,this.y=e,this}setScalar(t){return this.x=t,this.y=t,this}setX(t){return this.x=t,this}setY(t){return this.y=t,this}setComponent(t,e){switch(t){case 0:this.x=e;break;case 1:this.y=e;break;default:throw Error("THREE.Vector2: index is out of range: "+t)}return this}getComponent(t){switch(t){case 0:return this.x;case 1:return this.y;default:throw Error("THREE.Vector2: index is out of range: "+t)}}clone(){return new this.constructor(this.x,this.y)}copy(t){return this.x=t.x,this.y=t.y,this}add(t){return this.x+=t.x,this.y+=t.y,this}addScalar(t){return this.x+=t,this.y+=t,this}addVectors(t,e){return this.x=t.x+e.x,this.y=t.y+e.y,this}addScaledVector(t,e){return this.x+=t.x*e,this.y+=t.y*e,this}sub(t){return this.x-=t.x,this.y-=t.y,this}subScalar(t){return this.x-=t,this.y-=t,this}subVectors(t,e){return this.x=t.x-e.x,this.y=t.y-e.y,this}multiply(t){return this.x*=t.x,this.y*=t.y,this}multiplyScalar(t){return this.x*=t,this.y*=t,this}divide(t){return this.x/=t.x,this.y/=t.y,this}divideScalar(t){return this.multiplyScalar(1/t)}applyMatrix3(t){let e=this.x,n=this.y,i=t.elements;return this.x=i[0]*e+i[3]*n+i[6],this.y=i[1]*e+i[4]*n+i[7],this}min(t){return this.x=Math.min(this.x,t.x),this.y=Math.min(this.y,t.y),this}max(t){return this.x=Math.max(this.x,t.x),this.y=Math.max(this.y,t.y),this}clamp(t,e){return this.x=oe(this.x,t.x,e.x),this.y=oe(this.y,t.y,e.y),this}clampScalar(t,e){return this.x=oe(this.x,t,e),this.y=oe(this.y,t,e),this}clampLength(t,e){let n=this.length();return this.divideScalar(n||1).multiplyScalar(oe(n,t,e))}floor(){return this.x=Math.floor(this.x),this.y=Math.floor(this.y),this}ceil(){return this.x=Math.ceil(this.x),this.y=Math.ceil(this.y),this}round(){return this.x=Math.round(this.x),this.y=Math.round(this.y),this}roundToZero(){return this.x=Math.trunc(this.x),this.y=Math.trunc(this.y),this}negate(){return this.x=-this.x,this.y=-this.y,this}dot(t){return this.x*t.x+this.y*t.y}cross(t){return this.x*t.y-this.y*t.x}lengthSq(){return this.x*this.x+this.y*this.y}length(){return Math.sqrt(this.x*this.x+this.y*this.y)}manhattanLength(){return Math.abs(this.x)+Math.abs(this.y)}normalize(){return this.divideScalar(this.length()||1)}angle(){return Math.atan2(-this.y,-this.x)+Math.PI}angleTo(t){let e=Math.sqrt(this.lengthSq()*t.lengthSq());if(e===0)return Math.PI/2;let n=this.dot(t)/e;return Math.acos(oe(n,-1,1))}distanceTo(t){return Math.sqrt(this.distanceToSquared(t))}distanceToSquared(t){let e=this.x-t.x,n=this.y-t.y;return e*e+n*n}manhattanDistanceTo(t){return Math.abs(this.x-t.x)+Math.abs(this.y-t.y)}setLength(t){return this.normalize().multiplyScalar(t)}lerp(t,e){return this.x+=(t.x-this.x)*e,this.y+=(t.y-this.y)*e,this}lerpVectors(t,e,n){return this.x=t.x+(e.x-t.x)*n,this.y=t.y+(e.y-t.y)*n,this}equals(t){return t.x===this.x&&t.y===this.y}fromArray(t,e=0){return this.x=t[e],this.y=t[e+1],this}toArray(t=[],e=0){return t[e]=this.x,t[e+1]=this.y,t}fromBufferAttribute(t,e){return this.x=t.getX(e),this.y=t.getY(e),this}rotateAround(t,e){let n=Math.cos(e),i=Math.sin(e),s=this.x-t.x,r=this.y-t.y;return this.x=s*n-r*i+t.x,this.y=s*i+r*n+t.y,this}random(){return this.x=Math.random(),this.y=Math.random(),this}*[Symbol.iterator](){yield this.x,yield this.y}}class Bn{constructor(t=0,e=0,n=0,i=1){this.isQuaternion=!0,this._x=t,this._y=e,this._z=n,this._w=i}static slerpFlat(t,e,n,i,s,r,a){let o=n[i+0],l=n[i+1],c=n[i+2],p=n[i+3],h=s[r+0],u=s[r+1],d=s[r+2],v=s[r+3];if(p!==v||o!==h||l!==u||c!==d){let w=o*h+l*u+c*d+p*v;if(w<0)h=-h,u=-u,d=-d,v=-v,w=-w;let f=1-a;if(w<0.9995){let m=Math.acos(w),M=Math.sin(m);f=Math.sin(f*m)/M,a=Math.sin(a*m)/M,o=o*f+h*a,l=l*f+u*a,c=c*f+d*a,p=p*f+v*a}else{o=o*f+h*a,l=l*f+u*a,c=c*f+d*a,p=p*f+v*a;let m=1/Math.sqrt(o*o+l*l+c*c+p*p);o*=m,l*=m,c*=m,p*=m}}t[e]=o,t[e+1]=l,t[e+2]=c,t[e+3]=p}static multiplyQuaternionsFlat(t,e,n,i,s,r){let a=n[i],o=n[i+1],l=n[i+2],c=n[i+3],p=s[r],h=s[r+1],u=s[r+2],d=s[r+3];return t[e]=a*d+c*p+o*u-l*h,t[e+1]=o*d+c*h+l*p-a*u,t[e+2]=l*d+c*u+a*h-o*p,t[e+3]=c*d-a*p-o*h-l*u,t}get x(){return this._x}set x(t){this._x=t,this._onChangeCallback()}get y(){return this._y}set y(t){this._y=t,this._onChangeCallback()}get z(){return this._z}set z(t){this._z=t,this._onChangeCallback()}get w(){return this._w}set w(t){this._w=t,this._onChangeCallback()}set(t,e,n,i){return this._x=t,this._y=e,this._z=n,this._w=i,this._onChangeCallback(),this}clone(){return new this.constructor(this._x,this._y,this._z,this._w)}copy(t){return this._x=t.x,this._y=t.y,this._z=t.z,this._w=t.w,this._onChangeCallback(),this}setFromEuler(t,e=!0){let{_x:n,_y:i,_z:s,_order:r}=t,{cos:a,sin:o}=Math,l=a(n/2),c=a(i/2),p=a(s/2),h=o(n/2),u=o(i/2),d=o(s/2);switch(r){case"XYZ":this._x=h*c*p+l*u*d,this._y=l*u*p-h*c*d,this._z=l*c*d+h*u*p,this._w=l*c*p-h*u*d;break;case"YXZ":this._x=h*c*p+l*u*d,this._y=l*u*p-h*c*d,this._z=l*c*d-h*u*p,this._w=l*c*p+h*u*d;break;case"ZXY":this._x=h*c*p-l*u*d,this._y=l*u*p+h*c*d,this._z=l*c*d+h*u*p,this._w=l*c*p-h*u*d;break;case"ZYX":this._x=h*c*p-l*u*d,this._y=l*u*p+h*c*d,this._z=l*c*d-h*u*p,this._w=l*c*p+h*u*d;break;case"YZX":this._x=h*c*p+l*u*d,this._y=l*u*p+h*c*d,this._z=l*c*d-h*u*p,this._w=l*c*p-h*u*d;break;case"XZY":this._x=h*c*p-l*u*d,this._y=l*u*p-h*c*d,this._z=l*c*d+h*u*p,this._w=l*c*p+h*u*d;break;default:Wt("Quaternion: .setFromEuler() encountered an unknown order: "+r)}if(e===!0)this._onChangeCallback();return this}setFromAxisAngle(t,e){let n=e/2,i=Math.sin(n);return this._x=t.x*i,this._y=t.y*i,this._z=t.z*i,this._w=Math.cos(n),this._onChangeCallback(),this}setFromRotationMatrix(t){let e=t.elements,n=e[0],i=e[4],s=e[8],r=e[1],a=e[5],o=e[9],l=e[2],c=e[6],p=e[10],h=n+a+p;if(h>0){let u=0.5/Math.sqrt(h+1);this._w=0.25/u,this._x=(c-o)*u,this._y=(s-l)*u,this._z=(r-i)*u}else if(n>a&&n>p){let u=2*Math.sqrt(1+n-a-p);this._w=(c-o)/u,this._x=0.25*u,this._y=(i+r)/u,this._z=(s+l)/u}else if(a>p){let u=2*Math.sqrt(1+a-n-p);this._w=(s-l)/u,this._x=(i+r)/u,this._y=0.25*u,this._z=(o+c)/u}else{let u=2*Math.sqrt(1+p-n-a);this._w=(r-i)/u,this._x=(s+l)/u,this._y=(o+c)/u,this._z=0.25*u}return this._onChangeCallback(),this}setFromUnitVectors(t,e){let n=t.dot(e)+1;if(n<0.00000001)if(n=0,Math.abs(t.x)>Math.abs(t.z))this._x=-t.y,this._y=t.x,this._z=0,this._w=n;else this._x=0,this._y=-t.z,this._z=t.y,this._w=n;else this._x=t.y*e.z-t.z*e.y,this._y=t.z*e.x-t.x*e.z,this._z=t.x*e.y-t.y*e.x,this._w=n;return this.normalize()}angleTo(t){return 2*Math.acos(Math.abs(oe(this.dot(t),-1,1)))}rotateTowards(t,e){let n=this.angleTo(t);if(n===0)return this;let i=Math.min(1,e/n);return this.slerp(t,i),this}identity(){return this.set(0,0,0,1)}invert(){return this.conjugate()}conjugate(){return this._x*=-1,this._y*=-1,this._z*=-1,this._onChangeCallback(),this}dot(t){return this._x*t._x+this._y*t._y+this._z*t._z+this._w*t._w}lengthSq(){return this._x*this._x+this._y*this._y+this._z*this._z+this._w*this._w}length(){return Math.sqrt(this._x*this._x+this._y*this._y+this._z*this._z+this._w*this._w)}normalize(){let t=this.length();if(t===0)this._x=0,this._y=0,this._z=0,this._w=1;else t=1/t,this._x=this._x*t,this._y=this._y*t,this._z=this._z*t,this._w=this._w*t;return this._onChangeCallback(),this}multiply(t){return this.multiplyQuaternions(this,t)}premultiply(t){return this.multiplyQuaternions(t,this)}multiplyQuaternions(t,e){let{_x:n,_y:i,_z:s,_w:r}=t,{_x:a,_y:o,_z:l,_w:c}=e;return this._x=n*c+r*a+i*l-s*o,this._y=i*c+r*o+s*a-n*l,this._z=s*c+r*l+n*o-i*a,this._w=r*c-n*a-i*o-s*l,this._onChangeCallback(),this}slerp(t,e){let{_x:n,_y:i,_z:s,_w:r}=t,a=this.dot(t);if(a<0)n=-n,i=-i,s=-s,r=-r,a=-a;let o=1-e;if(a<0.9995){let l=Math.acos(a),c=Math.sin(l);o=Math.sin(o*l)/c,e=Math.sin(e*l)/c,this._x=this._x*o+n*e,this._y=this._y*o+i*e,this._z=this._z*o+s*e,this._w=this._w*o+r*e,this._onChangeCallback()}else this._x=this._x*o+n*e,this._y=this._y*o+i*e,this._z=this._z*o+s*e,this._w=this._w*o+r*e,this.normalize();return this}slerpQuaternions(t,e,n){return this.copy(t).slerp(e,n)}random(){let t=2*Math.PI*Math.random(),e=2*Math.PI*Math.random(),n=Math.random(),i=Math.sqrt(1-n),s=Math.sqrt(n);return this.set(i*Math.sin(t),i*Math.cos(t),s*Math.sin(e),s*Math.cos(e))}equals(t){return t._x===this._x&&t._y===this._y&&t._z===this._z&&t._w===this._w}fromArray(t,e=0){return this._x=t[e],this._y=t[e+1],this._z=t[e+2],this._w=t[e+3],this._onChangeCallback(),this}toArray(t=[],e=0){return t[e]=this._x,t[e+1]=this._y,t[e+2]=this._z,t[e+3]=this._w,t}fromBufferAttribute(t,e){return this._x=t.getX(e),this._y=t.getY(e),this._z=t.getZ(e),this._w=t.getW(e),this._onChangeCallback(),this}toJSON(){return this.toArray()}_onChange(t){return this._onChangeCallback=t,this}_onChangeCallback(){}*[Symbol.iterator](){yield this._x,yield this._y,yield this._z,yield this._w}}class W{static{this.prototype.isVector3=!0}constructor(t=0,e=0,n=0){this.x=t,this.y=e,this.z=n}set(t,e,n){if(n===void 0)n=this.z;return this.x=t,this.y=e,this.z=n,this}setScalar(t){return this.x=t,this.y=t,this.z=t,this}setX(t){return this.x=t,this}setY(t){return this.y=t,this}setZ(t){return this.z=t,this}setComponent(t,e){switch(t){case 0:this.x=e;break;case 1:this.y=e;break;case 2:this.z=e;break;default:throw Error("THREE.Vector3: index is out of range: "+t)}return this}getComponent(t){switch(t){case 0:return this.x;case 1:return this.y;case 2:return this.z;default:throw Error("THREE.Vector3: index is out of range: "+t)}}clone(){return new this.constructor(this.x,this.y,this.z)}copy(t){return this.x=t.x,this.y=t.y,this.z=t.z,this}add(t){return this.x+=t.x,this.y+=t.y,this.z+=t.z,this}addScalar(t){return this.x+=t,this.y+=t,this.z+=t,this}addVectors(t,e){return this.x=t.x+e.x,this.y=t.y+e.y,this.z=t.z+e.z,this}addScaledVector(t,e){return this.x+=t.x*e,this.y+=t.y*e,this.z+=t.z*e,this}sub(t){return this.x-=t.x,this.y-=t.y,this.z-=t.z,this}subScalar(t){return this.x-=t,this.y-=t,this.z-=t,this}subVectors(t,e){return this.x=t.x-e.x,this.y=t.y-e.y,this.z=t.z-e.z,this}multiply(t){return this.x*=t.x,this.y*=t.y,this.z*=t.z,this}multiplyScalar(t){return this.x*=t,this.y*=t,this.z*=t,this}multiplyVectors(t,e){return this.x=t.x*e.x,this.y=t.y*e.y,this.z=t.z*e.z,this}applyEuler(t){return this.applyQuaternion(Jo.setFromEuler(t))}applyAxisAngle(t,e){return this.applyQuaternion(Jo.setFromAxisAngle(t,e))}applyMatrix3(t){let e=this.x,n=this.y,i=this.z,s=t.elements;return this.x=s[0]*e+s[3]*n+s[6]*i,this.y=s[1]*e+s[4]*n+s[7]*i,this.z=s[2]*e+s[5]*n+s[8]*i,this}applyNormalMatrix(t){return this.applyMatrix3(t).normalize()}applyMatrix4(t){let e=this.x,n=this.y,i=this.z,s=t.elements,r=1/(s[3]*e+s[7]*n+s[11]*i+s[15]);return this.x=(s[0]*e+s[4]*n+s[8]*i+s[12])*r,this.y=(s[1]*e+s[5]*n+s[9]*i+s[13])*r,this.z=(s[2]*e+s[6]*n+s[10]*i+s[14])*r,this}applyQuaternion(t){let e=this.x,n=this.y,i=this.z,{x:s,y:r,z:a,w:o}=t,l=2*(r*i-a*n),c=2*(a*e-s*i),p=2*(s*n-r*e);return this.x=e+o*l+r*p-a*c,this.y=n+o*c+a*l-s*p,this.z=i+o*p+s*c-r*l,this}project(t){return this.applyMatrix4(t.matrixWorldInverse).applyMatrix4(t.projectionMatrix)}unproject(t){return this.applyMatrix4(t.projectionMatrixInverse).applyMatrix4(t.matrixWorld)}transformDirection(t){let e=this.x,n=this.y,i=this.z,s=t.elements;return this.x=s[0]*e+s[4]*n+s[8]*i,this.y=s[1]*e+s[5]*n+s[9]*i,this.z=s[2]*e+s[6]*n+s[10]*i,this.normalize()}divide(t){return this.x/=t.x,this.y/=t.y,this.z/=t.z,this}divideScalar(t){return this.multiplyScalar(1/t)}min(t){return this.x=Math.min(this.x,t.x),this.y=Math.min(this.y,t.y),this.z=Math.min(this.z,t.z),this}max(t){return this.x=Math.max(this.x,t.x),this.y=Math.max(this.y,t.y),this.z=Math.max(this.z,t.z),this}clamp(t,e){return this.x=oe(this.x,t.x,e.x),this.y=oe(this.y,t.y,e.y),this.z=oe(this.z,t.z,e.z),this}clampScalar(t,e){return this.x=oe(this.x,t,e),this.y=oe(this.y,t,e),this.z=oe(this.z,t,e),this}clampLength(t,e){let n=this.length();return this.divideScalar(n||1).multiplyScalar(oe(n,t,e))}floor(){return this.x=Math.floor(this.x),this.y=Math.floor(this.y),this.z=Math.floor(this.z),this}ceil(){return this.x=Math.ceil(this.x),this.y=Math.ceil(this.y),this.z=Math.ceil(this.z),this}round(){return this.x=Math.round(this.x),this.y=Math.round(this.y),this.z=Math.round(this.z),this}roundToZero(){return this.x=Math.trunc(this.x),this.y=Math.trunc(this.y),this.z=Math.trunc(this.z),this}negate(){return this.x=-this.x,this.y=-this.y,this.z=-this.z,this}dot(t){return this.x*t.x+this.y*t.y+this.z*t.z}lengthSq(){return this.x*this.x+this.y*this.y+this.z*this.z}length(){return Math.sqrt(this.x*this.x+this.y*this.y+this.z*this.z)}manhattanLength(){return Math.abs(this.x)+Math.abs(this.y)+Math.abs(this.z)}normalize(){return this.divideScalar(this.length()||1)}setLength(t){return this.normalize().multiplyScalar(t)}lerp(t,e){return this.x+=(t.x-this.x)*e,this.y+=(t.y-this.y)*e,this.z+=(t.z-this.z)*e,this}lerpVectors(t,e,n){return this.x=t.x+(e.x-t.x)*n,this.y=t.y+(e.y-t.y)*n,this.z=t.z+(e.z-t.z)*n,this}cross(t){return this.crossVectors(this,t)}crossVectors(t,e){let{x:n,y:i,z:s}=t,{x:r,y:a,z:o}=e;return this.x=i*o-s*a,this.y=s*r-n*o,this.z=n*a-i*r,this}projectOnVector(t){let e=t.lengthSq();if(e===0)return this.set(0,0,0);let n=t.dot(this)/e;return this.copy(t).multiplyScalar(n)}projectOnPlane(t){return Br.copy(this).projectOnVector(t),this.sub(Br)}reflect(t){return this.sub(Br.copy(t).multiplyScalar(2*this.dot(t)))}angleTo(t){let e=Math.sqrt(this.lengthSq()*t.lengthSq());if(e===0)return Math.PI/2;let n=this.dot(t)/e;return Math.acos(oe(n,-1,1))}distanceTo(t){return Math.sqrt(this.distanceToSquared(t))}distanceToSquared(t){let e=this.x-t.x,n=this.y-t.y,i=this.z-t.z;return e*e+n*n+i*i}manhattanDistanceTo(t){return Math.abs(this.x-t.x)+Math.abs(this.y-t.y)+Math.abs(this.z-t.z)}setFromSpherical(t){return this.setFromSphericalCoords(t.radius,t.phi,t.theta)}setFromSphericalCoords(t,e,n){let i=Math.sin(e)*t;return this.x=i*Math.sin(n),this.y=Math.cos(e)*t,this.z=i*Math.cos(n),this}setFromCylindrical(t){return this.setFromCylindricalCoords(t.radius,t.theta,t.y)}setFromCylindricalCoords(t,e,n){return this.x=t*Math.sin(e),this.y=n,this.z=t*Math.cos(e),this}setFromMatrixPosition(t){let e=t.elements;return this.x=e[12],this.y=e[13],this.z=e[14],this}setFromMatrixScale(t){let e=this.setFromMatrixColumn(t,0).length(),n=this.setFromMatrixColumn(t,1).length(),i=this.setFromMatrixColumn(t,2).length();return this.x=e,this.y=n,this.z=i,this}setFromMatrixColumn(t,e){return this.fromArray(t.elements,e*4)}setFromMatrix3Column(t,e){return this.fromArray(t.elements,e*3)}setFromEuler(t){return this.x=t._x,this.y=t._y,this.z=t._z,this}setFromColor(t){return this.x=t.r,this.y=t.g,this.z=t.b,this}equals(t){return t.x===this.x&&t.y===this.y&&t.z===this.z}fromArray(t,e=0){return this.x=t[e],this.y=t[e+1],this.z=t[e+2],this}toArray(t=[],e=0){return t[e]=this.x,t[e+1]=this.y,t[e+2]=this.z,t}fromBufferAttribute(t,e){return this.x=t.getX(e),this.y=t.getY(e),this.z=t.getZ(e),this}random(){return this.x=Math.random(),this.y=Math.random(),this.z=Math.random(),this}randomDirection(){let t=Math.random()*Math.PI*2,e=Math.random()*2-1,n=Math.sqrt(1-e*e);return this.x=n*Math.cos(t),this.y=e,this.z=n*Math.sin(t),this}*[Symbol.iterator](){yield this.x,yield this.y,yield this.z}}var Br=new W,Jo=new Bn;class Zt{static{this.prototype.isMatrix3=!0}constructor(t,e,n,i,s,r,a,o,l){if(this.elements=[1,0,0,0,1,0,0,0,1],t!==void 0)this.set(t,e,n,i,s,r,a,o,l)}set(t,e,n,i,s,r,a,o,l){let c=this.elements;return c[0]=t,c[1]=i,c[2]=a,c[3]=e,c[4]=s,c[5]=o,c[6]=n,c[7]=r,c[8]=l,this}identity(){return this.set(1,0,0,0,1,0,0,0,1),this}copy(t){let e=this.elements,n=t.elements;return e[0]=n[0],e[1]=n[1],e[2]=n[2],e[3]=n[3],e[4]=n[4],e[5]=n[5],e[6]=n[6],e[7]=n[7],e[8]=n[8],this}extractBasis(t,e,n){return t.setFromMatrix3Column(this,0),e.setFromMatrix3Column(this,1),n.setFromMatrix3Column(this,2),this}setFromMatrix4(t){let e=t.elements;return this.set(e[0],e[4],e[8],e[1],e[5],e[9],e[2],e[6],e[10]),this}multiply(t){return this.multiplyMatrices(this,t)}premultiply(t){return this.multiplyMatrices(t,this)}multiplyMatrices(t,e){let n=t.elements,i=e.elements,s=this.elements,r=n[0],a=n[3],o=n[6],l=n[1],c=n[4],p=n[7],h=n[2],u=n[5],d=n[8],v=i[0],w=i[3],f=i[6],m=i[1],M=i[4],T=i[7],S=i[2],E=i[5],P=i[8];return s[0]=r*v+a*m+o*S,s[3]=r*w+a*M+o*E,s[6]=r*f+a*T+o*P,s[1]=l*v+c*m+p*S,s[4]=l*w+c*M+p*E,s[7]=l*f+c*T+p*P,s[2]=h*v+u*m+d*S,s[5]=h*w+u*M+d*E,s[8]=h*f+u*T+d*P,this}multiplyScalar(t){let e=this.elements;return e[0]*=t,e[3]*=t,e[6]*=t,e[1]*=t,e[4]*=t,e[7]*=t,e[2]*=t,e[5]*=t,e[8]*=t,this}determinant(){let t=this.elements,e=t[0],n=t[1],i=t[2],s=t[3],r=t[4],a=t[5],o=t[6],l=t[7],c=t[8];return e*r*c-e*a*l-n*s*c+n*a*o+i*s*l-i*r*o}invert(){let t=this.elements,e=t[0],n=t[1],i=t[2],s=t[3],r=t[4],a=t[5],o=t[6],l=t[7],c=t[8],p=c*r-a*l,h=a*o-c*s,u=l*s-r*o,d=e*p+n*h+i*u;if(d===0)return this.set(0,0,0,0,0,0,0,0,0);let v=1/d;return t[0]=p*v,t[1]=(i*l-c*n)*v,t[2]=(a*n-i*r)*v,t[3]=h*v,t[4]=(c*e-i*o)*v,t[5]=(i*s-a*e)*v,t[6]=u*v,t[7]=(n*o-l*e)*v,t[8]=(r*e-n*s)*v,this}transpose(){let t,e=this.elements;return t=e[1],e[1]=e[3],e[3]=t,t=e[2],e[2]=e[6],e[6]=t,t=e[5],e[5]=e[7],e[7]=t,this}getNormalMatrix(t){return this.setFromMatrix4(t).invert().transpose()}transposeIntoArray(t){let e=this.elements;return t[0]=e[0],t[1]=e[3],t[2]=e[6],t[3]=e[1],t[4]=e[4],t[5]=e[7],t[6]=e[2],t[7]=e[5],t[8]=e[8],this}setUvTransform(t,e,n,i,s,r,a){let o=Math.cos(s),l=Math.sin(s);return this.set(n*o,n*l,-n*(o*r+l*a)+r+t,-i*l,i*o,-i*(-l*r+o*a)+a+e,0,0,1),this}scale(t,e){return es("Matrix3: .scale() is deprecated. Use .makeScale() instead."),this.premultiply(zr.makeScale(t,e)),this}rotate(t){return es("Matrix3: .rotate() is deprecated. Use .makeRotation() instead."),this.premultiply(zr.makeRotation(-t)),this}translate(t,e){return es("Matrix3: .translate() is deprecated. Use .makeTranslation() instead."),this.premultiply(zr.makeTranslation(t,e)),this}makeTranslation(t,e){if(t.isVector2)this.set(1,0,t.x,0,1,t.y,0,0,1);else this.set(1,0,t,0,1,e,0,0,1);return this}makeRotation(t){let e=Math.cos(t),n=Math.sin(t);return this.set(e,-n,0,n,e,0,0,0,1),this}makeScale(t,e){return this.set(t,0,0,0,e,0,0,0,1),this}equals(t){let e=this.elements,n=t.elements;for(let i=0;i<9;i++)if(e[i]!==n[i])return!1;return!0}fromArray(t,e=0){for(let n=0;n<9;n++)this.elements[n]=t[n+e];return this}toArray(t=[],e=0){let n=this.elements;return t[e]=n[0],t[e+1]=n[1],t[e+2]=n[2],t[e+3]=n[3],t[e+4]=n[4],t[e+5]=n[5],t[e+6]=n[6],t[e+7]=n[7],t[e+8]=n[8],t}clone(){return new this.constructor().fromArray(this.elements)}}var zr=new Zt,$o=new Zt().set(0.4123908,0.3575843,0.1804808,0.212639,0.7151687,0.0721923,0.0193308,0.1191948,0.9505322),Ko=new Zt().set(3.2409699,-1.5373832,-0.4986108,-0.9692436,1.8759675,0.0415551,0.0556301,-0.203977,1.0569715);function Ih(){let t={enabled:!0,workingColorSpace:"srgb-linear",spaces:{},convert:function(s,r,a){if(this.enabled===!1||r===a||!r||!a)return s;if(this.spaces[r].transfer==="srgb")s.r=Dn(s.r),s.g=Dn(s.g),s.b=Dn(s.b);if(this.spaces[r].primaries!==this.spaces[a].primaries)s.applyMatrix3(this.spaces[r].toXYZ),s.applyMatrix3(this.spaces[a].fromXYZ);if(this.spaces[a].transfer==="srgb")s.r=Di(s.r),s.g=Di(s.g),s.b=Di(s.b);return s},workingToColorSpace:function(s,r){return this.convert(s,this.workingColorSpace,r)},colorSpaceToWorking:function(s,r){return this.convert(s,r,this.workingColorSpace)},getPrimaries:function(s){return this.spaces[s].primaries},getTransfer:function(s){if(s==="")return"linear";return this.spaces[s].transfer},getToneMappingMode:function(s){return this.spaces[s].outputColorSpaceConfig.toneMappingMode||"standard"},getLuminanceCoefficients:function(s,r=this.workingColorSpace){return s.fromArray(this.spaces[r].luminanceCoefficients)},define:function(s){Object.assign(this.spaces,s)},_getMatrix:function(s,r,a){return s.copy(this.spaces[a].fromXYZ).multiply(this.spaces[r].toXYZ)},_getDrawingBufferColorSpace:function(s){return this.spaces[s].outputColorSpaceConfig.drawingBufferColorSpace},_getUnpackColorSpace:function(s=this.workingColorSpace){return this.spaces[s].workingColorSpaceConfig.unpackColorSpace}},e=[0.64,0.33,0.3,0.6,0.15,0.06],n=[0.2126,0.7152,0.0722],i=[0.3127,0.329];return t.define({["srgb-linear"]:{primaries:e,whitePoint:i,transfer:"linear",toXYZ:$o,fromXYZ:Ko,luminanceCoefficients:n,workingColorSpaceConfig:{unpackColorSpace:"srgb"},outputColorSpaceConfig:{drawingBufferColorSpace:"srgb"}},["srgb"]:{primaries:e,whitePoint:i,transfer:"srgb",toXYZ:$o,fromXYZ:Ko,luminanceCoefficients:n,outputColorSpaceConfig:{drawingBufferColorSpace:"srgb"}}}),t}var ae=Ih();function Dn(t){return t<0.04045?t*0.0773993808:Math.pow(t*0.9478672986+0.0521327014,2.4)}function Di(t){return t<0.0031308?t*12.92:1.055*Math.pow(t,0.41666)-0.055}var yi;class lo{static getDataURL(t,e="image/png"){if(/^data:/i.test(t.src))return t.src;if(typeof HTMLCanvasElement>"u")return t.src;let n;if(t instanceof HTMLCanvasElement)n=t;else{if(yi===void 0)yi=ns("canvas");yi.width=t.width,yi.height=t.height;let i=yi.getContext("2d");if(t instanceof ImageData)i.putImageData(t,0,0);else i.drawImage(t,0,0,t.width,t.height);n=yi}return n.toDataURL(e)}static sRGBToLinear(t){if(typeof HTMLImageElement<"u"&&t instanceof HTMLImageElement||typeof HTMLCanvasElement<"u"&&t instanceof HTMLCanvasElement||typeof ImageBitmap<"u"&&t instanceof ImageBitmap){let e=ns("canvas");e.width=t.width,e.height=t.height;let n=e.getContext("2d");n.drawImage(t,0,0,t.width,t.height);let i=n.getImageData(0,0,t.width,t.height),s=i.data;for(let r=0;r<s.length;r++)s[r]=Dn(s[r]/255)*255;return n.putImageData(i,0,0),e}else if(t.data){let e=t.data.slice(0);for(let n=0;n<e.length;n++)if(e instanceof Uint8Array||e instanceof Uint8ClampedArray)e[n]=Math.floor(Dn(e[n]/255)*255);else e[n]=Dn(e[n]);return{data:e,width:t.width,height:t.height}}else return Wt("ImageUtils.sRGBToLinear(): Unsupported image type. No color space conversion applied."),t}}var Ph=0;class cs{constructor(t=null){this.isTextureSource=!0,Object.defineProperty(this,"id",{value:Ph++}),this.uuid=ls(),this.data=t,this.dataReady=!0,this.version=0}getSize(t){let e=this.data;if(typeof HTMLVideoElement<"u"&&e instanceof HTMLVideoElement)t.set(e.videoWidth,e.videoHeight,0);else if(typeof VideoFrame<"u"&&e instanceof VideoFrame)t.set(e.displayWidth,e.displayHeight,0);else if(e!==null)t.set(e.width||0,e.height||0,e.depth||0);else t.set(0,0,0);return t}set needsUpdate(t){if(t===!0)this.version++}toJSON(t){let e=t===void 0||typeof t==="string";if(!e&&t.images[this.uuid]!==void 0)return t.images[this.uuid];let n={uuid:this.uuid,url:""},i=this.data;if(i!==null){let s;if(Array.isArray(i)){s=[];for(let r=0,a=i.length;r<a;r++)if(i[r].isDataTexture)s.push(kr(i[r].image));else s.push(kr(i[r]))}else s=kr(i);n.url=s}if(!e)t.images[this.uuid]=n;return n}}function kr(t){if(typeof HTMLImageElement<"u"&&t instanceof HTMLImageElement||typeof HTMLCanvasElement<"u"&&t instanceof HTMLCanvasElement||typeof ImageBitmap<"u"&&t instanceof ImageBitmap)return lo.getDataURL(t);else if(t.data)return{data:Array.from(t.data),width:t.width,height:t.height,type:t.data.constructor.name};else return Wt("Texture: Unable to serialize Texture."),{}}var Lh=0,Gr=new W;class Ve extends On{constructor(t=Ve.DEFAULT_IMAGE,e=Ve.DEFAULT_MAPPING,n=1001,i=1001,s=1006,r=1008,a=1023,o=1009,l=Ve.DEFAULT_ANISOTROPY,c=""){super();this.isTexture=!0,Object.defineProperty(this,"id",{value:Lh++}),this.uuid=ls(),this.name="",this.source=new cs(t),this.mipmaps=[],this.mapping=e,this.channel=0,this.wrapS=n,this.wrapT=i,this.magFilter=s,this.minFilter=r,this.anisotropy=l,this.format=a,this.internalFormat=null,this.type=o,this.offset=new fe(0,0),this.repeat=new fe(1,1),this.center=new fe(0,0),this.rotation=0,this.matrixAutoUpdate=!0,this.matrix=new Zt,this.generateMipmaps=!0,this.mipmapsAutoUpdate=!0,this.premultiplyAlpha=!1,this.flipY=!0,this.unpackAlignment=4,this.colorSpace=c,this.userData={},this.updateRanges=[],this.version=0,this.onUpdate=null,this.renderTarget=null,this.isRenderTargetTexture=!1,this.isArrayTexture=t&&t.depth&&t.depth>1?!0:!1,this.pmremVersion=0,this.isPMREMTexture=!1,this.normalized=!1}get width(){return this.source.getSize(Gr).x}get height(){return this.source.getSize(Gr).y}get depth(){return this.source.getSize(Gr).z}get image(){return this.source.data}set image(t){this.source.data=t}updateMatrix(){this.matrix.setUvTransform(this.offset.x,this.offset.y,this.repeat.x,this.repeat.y,this.rotation,this.center.x,this.center.y)}addUpdateRange(t,e){this.updateRanges.push({start:t,count:e})}clearUpdateRanges(){this.updateRanges.length=0}clone(){return new this.constructor().copy(this)}copy(t){return this.name=t.name,this.source=t.source,this.mipmaps=t.mipmaps.slice(0),this.mapping=t.mapping,this.channel=t.channel,this.wrapS=t.wrapS,this.wrapT=t.wrapT,this.magFilter=t.magFilter,this.minFilter=t.minFilter,this.anisotropy=t.anisotropy,this.format=t.format,this.internalFormat=t.internalFormat,this.type=t.type,this.normalized=t.normalized,this.offset.copy(t.offset),this.repeat.copy(t.repeat),this.center.copy(t.center),this.rotation=t.rotation,this.matrixAutoUpdate=t.matrixAutoUpdate,this.matrix.copy(t.matrix),this.generateMipmaps=t.generateMipmaps,this.mipmapsAutoUpdate=t.mipmapsAutoUpdate,this.premultiplyAlpha=t.premultiplyAlpha,this.flipY=t.flipY,this.unpackAlignment=t.unpackAlignment,this.colorSpace=t.colorSpace,this.renderTarget=t.renderTarget,this.isRenderTargetTexture=t.isRenderTargetTexture,this.isPMREMTexture=t.isPMREMTexture,this.isArrayTexture=t.isArrayTexture,this.userData=JSON.parse(JSON.stringify(t.userData)),this.needsUpdate=!0,this}setValues(t){for(let e in t){let n=t[e];if(n===void 0){Wt(`Texture.setValues(): parameter '${e}' has value of undefined.`);continue}let i=this[e];if(i===void 0){Wt(`Texture.setValues(): property '${e}' does not exist.`);continue}if(i&&n&&(i.isVector2&&n.isVector2))i.copy(n);else if(i&&n&&(i.isVector3&&n.isVector3))i.copy(n);else if(i&&n&&(i.isMatrix3&&n.isMatrix3))i.copy(n);else this[e]=n}}toJSON(t){let e=t===void 0||typeof t==="string";if(!e&&t.textures[this.uuid]!==void 0)return t.textures[this.uuid];let n={metadata:{version:4.7,type:"Texture",generator:"Texture.toJSON"},uuid:this.uuid,name:this.name,image:this.source.toJSON(t).uuid,mapping:this.mapping,channel:this.channel,repeat:[this.repeat.x,this.repeat.y],offset:[this.offset.x,this.offset.y],center:[this.center.x,this.center.y],rotation:this.rotation,wrap:[this.wrapS,this.wrapT],format:this.format,internalFormat:this.internalFormat,type:this.type,normalized:this.normalized,colorSpace:this.colorSpace,minFilter:this.minFilter,magFilter:this.magFilter,anisotropy:this.anisotropy,flipY:this.flipY,generateMipmaps:this.generateMipmaps,mipmapsAutoUpdate:this.mipmapsAutoUpdate,premultiplyAlpha:this.premultiplyAlpha,unpackAlignment:this.unpackAlignment};if(Object.keys(this.userData).length>0)n.userData=this.userData;if(!e)t.textures[this.uuid]=n;return n}dispose(){this.dispatchEvent({type:"dispose"})}transformUv(t){if(this.mapping!==300)return t;if(t.applyMatrix3(this.matrix),t.x<0||t.x>1)switch(this.wrapS){case 1000:t.x=t.x-Math.floor(t.x);break;case 1001:t.x=t.x<0?0:1;break;case 1002:if(Math.abs(Math.floor(t.x)%2)===1)t.x=Math.ceil(t.x)-t.x;else t.x=t.x-Math.floor(t.x);break}if(t.y<0||t.y>1)switch(this.wrapT){case 1000:t.y=t.y-Math.floor(t.y);break;case 1001:t.y=t.y<0?0:1;break;case 1002:if(Math.abs(Math.floor(t.y)%2)===1)t.y=Math.ceil(t.y)-t.y;else t.y=t.y-Math.floor(t.y);break}if(this.flipY)t.y=1-t.y;return t}set needsUpdate(t){if(t===!0)this.version++,this.source.needsUpdate=!0}set needsPMREMUpdate(t){if(t===!0)this.pmremVersion++}}Ve.DEFAULT_IMAGE=null;Ve.DEFAULT_MAPPING=300;Ve.DEFAULT_ANISOTROPY=1;class Ae{static{this.prototype.isVector4=!0}constructor(t=0,e=0,n=0,i=1){this.x=t,this.y=e,this.z=n,this.w=i}get width(){return this.z}set width(t){this.z=t}get height(){return this.w}set height(t){this.w=t}set(t,e,n,i){return this.x=t,this.y=e,this.z=n,this.w=i,this}setScalar(t){return this.x=t,this.y=t,this.z=t,this.w=t,this}setX(t){return this.x=t,this}setY(t){return this.y=t,this}setZ(t){return this.z=t,this}setW(t){return this.w=t,this}setComponent(t,e){switch(t){case 0:this.x=e;break;case 1:this.y=e;break;case 2:this.z=e;break;case 3:this.w=e;break;default:throw Error("THREE.Vector4: index is out of range: "+t)}return this}getComponent(t){switch(t){case 0:return this.x;case 1:return this.y;case 2:return this.z;case 3:return this.w;default:throw Error("THREE.Vector4: index is out of range: "+t)}}clone(){return new this.constructor(this.x,this.y,this.z,this.w)}copy(t){return this.x=t.x,this.y=t.y,this.z=t.z,this.w=t.w!==void 0?t.w:1,this}add(t){return this.x+=t.x,this.y+=t.y,this.z+=t.z,this.w+=t.w,this}addScalar(t){return this.x+=t,this.y+=t,this.z+=t,this.w+=t,this}addVectors(t,e){return this.x=t.x+e.x,this.y=t.y+e.y,this.z=t.z+e.z,this.w=t.w+e.w,this}addScaledVector(t,e){return this.x+=t.x*e,this.y+=t.y*e,this.z+=t.z*e,this.w+=t.w*e,this}sub(t){return this.x-=t.x,this.y-=t.y,this.z-=t.z,this.w-=t.w,this}subScalar(t){return this.x-=t,this.y-=t,this.z-=t,this.w-=t,this}subVectors(t,e){return this.x=t.x-e.x,this.y=t.y-e.y,this.z=t.z-e.z,this.w=t.w-e.w,this}multiply(t){return this.x*=t.x,this.y*=t.y,this.z*=t.z,this.w*=t.w,this}multiplyScalar(t){return this.x*=t,this.y*=t,this.z*=t,this.w*=t,this}applyMatrix4(t){let e=this.x,n=this.y,i=this.z,s=this.w,r=t.elements;return this.x=r[0]*e+r[4]*n+r[8]*i+r[12]*s,this.y=r[1]*e+r[5]*n+r[9]*i+r[13]*s,this.z=r[2]*e+r[6]*n+r[10]*i+r[14]*s,this.w=r[3]*e+r[7]*n+r[11]*i+r[15]*s,this}divide(t){return this.x/=t.x,this.y/=t.y,this.z/=t.z,this.w/=t.w,this}divideScalar(t){return this.multiplyScalar(1/t)}setAxisAngleFromQuaternion(t){this.w=2*Math.acos(t.w);let e=Math.sqrt(1-t.w*t.w);if(e<0.0001)this.x=1,this.y=0,this.z=0;else this.x=t.x/e,this.y=t.y/e,this.z=t.z/e;return this}setAxisAngleFromRotationMatrix(t){let e,n,i,s,r=0.01,a=0.1,o=t.elements,l=o[0],c=o[4],p=o[8],h=o[1],u=o[5],d=o[9],v=o[2],w=o[6],f=o[10];if(Math.abs(c-h)<0.01&&Math.abs(p-v)<0.01&&Math.abs(d-w)<0.01){if(Math.abs(c+h)<0.1&&Math.abs(p+v)<0.1&&Math.abs(d+w)<0.1&&Math.abs(l+u+f-3)<0.1)return this.set(1,0,0,0),this;e=Math.PI;let M=(l+1)/2,T=(u+1)/2,S=(f+1)/2,E=(c+h)/4,P=(p+v)/4,g=(d+w)/4;if(M>T&&M>S)if(M<0.01)n=0,i=0.707106781,s=0.707106781;else n=Math.sqrt(M),i=E/n,s=P/n;else if(T>S)if(T<0.01)n=0.707106781,i=0,s=0.707106781;else i=Math.sqrt(T),n=E/i,s=g/i;else if(S<0.01)n=0.707106781,i=0.707106781,s=0;else s=Math.sqrt(S),n=P/s,i=g/s;return this.set(n,i,s,e),this}let m=Math.sqrt((w-d)*(w-d)+(p-v)*(p-v)+(h-c)*(h-c));if(Math.abs(m)<0.001)m=1;return this.x=(w-d)/m,this.y=(p-v)/m,this.z=(h-c)/m,this.w=Math.acos((l+u+f-1)/2),this}setFromMatrixPosition(t){let e=t.elements;return this.x=e[12],this.y=e[13],this.z=e[14],this.w=e[15],this}min(t){return this.x=Math.min(this.x,t.x),this.y=Math.min(this.y,t.y),this.z=Math.min(this.z,t.z),this.w=Math.min(this.w,t.w),this}max(t){return this.x=Math.max(this.x,t.x),this.y=Math.max(this.y,t.y),this.z=Math.max(this.z,t.z),this.w=Math.max(this.w,t.w),this}clamp(t,e){return this.x=oe(this.x,t.x,e.x),this.y=oe(this.y,t.y,e.y),this.z=oe(this.z,t.z,e.z),this.w=oe(this.w,t.w,e.w),this}clampScalar(t,e){return this.x=oe(this.x,t,e),this.y=oe(this.y,t,e),this.z=oe(this.z,t,e),this.w=oe(this.w,t,e),this}clampLength(t,e){let n=this.length();return this.divideScalar(n||1).multiplyScalar(oe(n,t,e))}floor(){return this.x=Math.floor(this.x),this.y=Math.floor(this.y),this.z=Math.floor(this.z),this.w=Math.floor(this.w),this}ceil(){return this.x=Math.ceil(this.x),this.y=Math.ceil(this.y),this.z=Math.ceil(this.z),this.w=Math.ceil(this.w),this}round(){return this.x=Math.round(this.x),this.y=Math.round(this.y),this.z=Math.round(this.z),this.w=Math.round(this.w),this}roundToZero(){return this.x=Math.trunc(this.x),this.y=Math.trunc(this.y),this.z=Math.trunc(this.z),this.w=Math.trunc(this.w),this}negate(){return this.x=-this.x,this.y=-this.y,this.z=-this.z,this.w=-this.w,this}dot(t){return this.x*t.x+this.y*t.y+this.z*t.z+this.w*t.w}lengthSq(){return this.x*this.x+this.y*this.y+this.z*this.z+this.w*this.w}length(){return Math.sqrt(this.x*this.x+this.y*this.y+this.z*this.z+this.w*this.w)}manhattanLength(){return Math.abs(this.x)+Math.abs(this.y)+Math.abs(this.z)+Math.abs(this.w)}normalize(){return this.divideScalar(this.length()||1)}setLength(t){return this.normalize().multiplyScalar(t)}lerp(t,e){return this.x+=(t.x-this.x)*e,this.y+=(t.y-this.y)*e,this.z+=(t.z-this.z)*e,this.w+=(t.w-this.w)*e,this}lerpVectors(t,e,n){return this.x=t.x+(e.x-t.x)*n,this.y=t.y+(e.y-t.y)*n,this.z=t.z+(e.z-t.z)*n,this.w=t.w+(e.w-t.w)*n,this}equals(t){return t.x===this.x&&t.y===this.y&&t.z===this.z&&t.w===this.w}fromArray(t,e=0){return this.x=t[e],this.y=t[e+1],this.z=t[e+2],this.w=t[e+3],this}toArray(t=[],e=0){return t[e]=this.x,t[e+1]=this.y,t[e+2]=this.z,t[e+3]=this.w,t}fromBufferAttribute(t,e){return this.x=t.getX(e),this.y=t.getY(e),this.z=t.getZ(e),this.w=t.getW(e),this}random(){return this.x=Math.random(),this.y=Math.random(),this.z=Math.random(),this.w=Math.random(),this}*[Symbol.iterator](){yield this.x,yield this.y,yield this.z,yield this.w}}class co extends On{constructor(t=1,e=1,n={}){super();n=Object.assign({generateMipmaps:!1,internalFormat:null,minFilter:1006,depthBuffer:!0,stencilBuffer:!1,resolveColorBuffer:!0,resolveDepthBuffer:!0,resolveStencilBuffer:!0,storeMultisampledColorBuffer:!0,storeMultisampledDepthBuffer:!0,storeMultisampledStencilBuffer:!0,depthTexture:null,samples:0,count:1,depth:1,multiview:!1,useArrayDepthTexture:!1},n),this.isRenderTarget=!0,this.width=t,this.height=e,this.depth=n.depth,this.scissor=new Ae(0,0,t,e),this.scissorTest=!1,this.viewport=new Ae(0,0,t,e),this.textures=[];let i=n.count;for(let s=0;s<i;s++)this.textures[s]=new Ve({width:t,height:e,depth:n.depth}),this.textures[s].isRenderTargetTexture=!0,this.textures[s].renderTarget=this;this._setTextureOptions(n),this.depthBuffer=n.depthBuffer,this.stencilBuffer=n.stencilBuffer,this.resolveColorBuffer=n.resolveColorBuffer,this.resolveDepthBuffer=n.resolveDepthBuffer,this.resolveStencilBuffer=n.resolveStencilBuffer,this.storeMultisampledColorBuffer=n.storeMultisampledColorBuffer,this.storeMultisampledDepthBuffer=n.storeMultisampledDepthBuffer,this.storeMultisampledStencilBuffer=n.storeMultisampledStencilBuffer,this._depthTexture=null,this.depthTexture=n.depthTexture,this.samples=n.samples,this.multiview=n.multiview,this.useArrayDepthTexture=n.useArrayDepthTexture}_setTextureOptions(t={}){let e={minFilter:1006,generateMipmaps:!1,flipY:!1,internalFormat:null};if(t.mapping!==void 0)e.mapping=t.mapping;if(t.wrapS!==void 0)e.wrapS=t.wrapS;if(t.wrapT!==void 0)e.wrapT=t.wrapT;if(t.wrapR!==void 0)e.wrapR=t.wrapR;if(t.magFilter!==void 0)e.magFilter=t.magFilter;if(t.minFilter!==void 0)e.minFilter=t.minFilter;if(t.format!==void 0)e.format=t.format;if(t.type!==void 0)e.type=t.type;if(t.anisotropy!==void 0)e.anisotropy=t.anisotropy;if(t.colorSpace!==void 0)e.colorSpace=t.colorSpace;if(t.flipY!==void 0)e.flipY=t.flipY;if(t.generateMipmaps!==void 0)e.generateMipmaps=t.generateMipmaps;if(t.mipmapsAutoUpdate!==void 0)e.mipmapsAutoUpdate=t.mipmapsAutoUpdate;if(t.internalFormat!==void 0)e.internalFormat=t.internalFormat;for(let n=0;n<this.textures.length;n++)this.textures[n].setValues(e)}get texture(){return this.textures[0]}set texture(t){this.textures[0]=t}set depthTexture(t){if(this._depthTexture!==null&&this._depthTexture.renderTarget===this)this._depthTexture.renderTarget=null;if(t!==null&&t.renderTarget===null)t.renderTarget=this;this._depthTexture=t}get depthTexture(){return this._depthTexture}setSize(t,e,n=1){if(this.width!==t||this.height!==e||this.depth!==n){this.width=t,this.height=e,this.depth=n;for(let i=0,s=this.textures.length;i<s;i++)if(this.textures[i].image.width=t,this.textures[i].image.height=e,this.textures[i].image.depth=n,this.textures[i].isData3DTexture!==!0)this.textures[i].isArrayTexture=this.textures[i].image.depth>1;this.dispose()}this.viewport.set(0,0,t,e),this.scissor.set(0,0,t,e)}clone(){return new this.constructor().copy(this)}copy(t){this.width=t.width,this.height=t.height,this.depth=t.depth,this.scissor.copy(t.scissor),this.scissorTest=t.scissorTest,this.viewport.copy(t.viewport),this.textures.length=0;for(let e=0,n=t.textures.length;e<n;e++){this.textures[e]=t.textures[e].clone(),this.textures[e].isRenderTargetTexture=!0,this.textures[e].renderTarget=this;let i=Object.assign({},t.textures[e].image);this.textures[e].source=new cs(i)}if(this.depthBuffer=t.depthBuffer,this.stencilBuffer=t.stencilBuffer,this.resolveColorBuffer=t.resolveColorBuffer,this.resolveDepthBuffer=t.resolveDepthBuffer,this.resolveStencilBuffer=t.resolveStencilBuffer,this.storeMultisampledColorBuffer=t.storeMultisampledColorBuffer,this.storeMultisampledDepthBuffer=t.storeMultisampledDepthBuffer,this.storeMultisampledStencilBuffer=t.storeMultisampledStencilBuffer,t.depthTexture!==null)if(t.depthTexture.renderTarget===t){let e=t.depthTexture.clone();e.renderTarget=null,this.depthTexture=e}else this.depthTexture=t.depthTexture;else this.depthTexture=null;return this.samples=t.samples,this.multiview=t.multiview,this.useArrayDepthTexture=t.useArrayDepthTexture,this}dispose(){this.dispatchEvent({type:"dispose"})}}class cn extends co{constructor(t=1,e=1,n={}){super(t,e,n);this.isWebGLRenderTarget=!0}}class cr extends Ve{constructor(t=null,e=1,n=1,i=1){super(null);this.isDataArrayTexture=!0,this.image={data:t,width:e,height:n,depth:i},this.magFilter=1003,this.minFilter=1003,this.wrapR=1001,this.generateMipmaps=!1,this.flipY=!1,this.unpackAlignment=1,this.layerUpdates=new Set}copy(t){return super.copy(t),this.wrapR=t.wrapR,this}addLayerUpdate(t){this.layerUpdates.add(t)}clearLayerUpdates(){this.layerUpdates.clear()}}class ho extends Ve{constructor(t=null,e=1,n=1,i=1){super(null);this.isData3DTexture=!0,this.image={data:t,width:e,height:n,depth:i},this.magFilter=1003,this.minFilter=1003,this.wrapR=1001,this.generateMipmaps=!1,this.flipY=!1,this.unpackAlignment=1}copy(t){return super.copy(t),this.wrapR=t.wrapR,this}}class Se{static{this.prototype.isMatrix4=!0}constructor(t,e,n,i,s,r,a,o,l,c,p,h,u,d,v,w){if(this.elements=[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1],t!==void 0)this.set(t,e,n,i,s,r,a,o,l,c,p,h,u,d,v,w)}set(t,e,n,i,s,r,a,o,l,c,p,h,u,d,v,w){let f=this.elements;return f[0]=t,f[4]=e,f[8]=n,f[12]=i,f[1]=s,f[5]=r,f[9]=a,f[13]=o,f[2]=l,f[6]=c,f[10]=p,f[14]=h,f[3]=u,f[7]=d,f[11]=v,f[15]=w,this}identity(){return this.set(1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1),this}clone(){return new Se().fromArray(this.elements)}copy(t){let e=this.elements,n=t.elements;return e[0]=n[0],e[1]=n[1],e[2]=n[2],e[3]=n[3],e[4]=n[4],e[5]=n[5],e[6]=n[6],e[7]=n[7],e[8]=n[8],e[9]=n[9],e[10]=n[10],e[11]=n[11],e[12]=n[12],e[13]=n[13],e[14]=n[14],e[15]=n[15],this}copyPosition(t){let e=this.elements,n=t.elements;return e[12]=n[12],e[13]=n[13],e[14]=n[14],this}setFromMatrix3(t){let e=t.elements;return this.set(e[0],e[3],e[6],0,e[1],e[4],e[7],0,e[2],e[5],e[8],0,0,0,0,1),this}extractBasis(t,e,n){if(this.determinantAffine()===0)return t.set(1,0,0),e.set(0,1,0),n.set(0,0,1),this;return t.setFromMatrixColumn(this,0),e.setFromMatrixColumn(this,1),n.setFromMatrixColumn(this,2),this}makeBasis(t,e,n){return this.set(t.x,e.x,n.x,0,t.y,e.y,n.y,0,t.z,e.z,n.z,0,0,0,0,1),this}extractRotation(t){if(t.determinantAffine()===0)return this.identity();let e=this.elements,n=t.elements,i=1/Mi.setFromMatrixColumn(t,0).length(),s=1/Mi.setFromMatrixColumn(t,1).length(),r=1/Mi.setFromMatrixColumn(t,2).length();return e[0]=n[0]*i,e[1]=n[1]*i,e[2]=n[2]*i,e[3]=0,e[4]=n[4]*s,e[5]=n[5]*s,e[6]=n[6]*s,e[7]=0,e[8]=n[8]*r,e[9]=n[9]*r,e[10]=n[10]*r,e[11]=0,e[12]=0,e[13]=0,e[14]=0,e[15]=1,this}makeRotationFromEuler(t){let e=this.elements,{x:n,y:i,z:s}=t,r=Math.cos(n),a=Math.sin(n),o=Math.cos(i),l=Math.sin(i),c=Math.cos(s),p=Math.sin(s);if(t.order==="XYZ"){let h=r*c,u=r*p,d=a*c,v=a*p;e[0]=o*c,e[4]=-o*p,e[8]=l,e[1]=u+d*l,e[5]=h-v*l,e[9]=-a*o,e[2]=v-h*l,e[6]=d+u*l,e[10]=r*o}else if(t.order==="YXZ"){let h=o*c,u=o*p,d=l*c,v=l*p;e[0]=h+v*a,e[4]=d*a-u,e[8]=r*l,e[1]=r*p,e[5]=r*c,e[9]=-a,e[2]=u*a-d,e[6]=v+h*a,e[10]=r*o}else if(t.order==="ZXY"){let h=o*c,u=o*p,d=l*c,v=l*p;e[0]=h-v*a,e[4]=-r*p,e[8]=d+u*a,e[1]=u+d*a,e[5]=r*c,e[9]=v-h*a,e[2]=-r*l,e[6]=a,e[10]=r*o}else if(t.order==="ZYX"){let h=r*c,u=r*p,d=a*c,v=a*p;e[0]=o*c,e[4]=d*l-u,e[8]=h*l+v,e[1]=o*p,e[5]=v*l+h,e[9]=u*l-d,e[2]=-l,e[6]=a*o,e[10]=r*o}else if(t.order==="YZX"){let h=r*o,u=r*l,d=a*o,v=a*l;e[0]=o*c,e[4]=v-h*p,e[8]=d*p+u,e[1]=p,e[5]=r*c,e[9]=-a*c,e[2]=-l*c,e[6]=u*p+d,e[10]=h-v*p}else if(t.order==="XZY"){let h=r*o,u=r*l,d=a*o,v=a*l;e[0]=o*c,e[4]=-p,e[8]=l*c,e[1]=h*p+v,e[5]=r*c,e[9]=u*p-d,e[2]=d*p-u,e[6]=a*c,e[10]=v*p+h}return e[3]=0,e[7]=0,e[11]=0,e[12]=0,e[13]=0,e[14]=0,e[15]=1,this}makeRotationFromQuaternion(t){return this.compose(Nh,t,Dh)}lookAt(t,e,n){let i=this.elements;if(en.subVectors(t,e),en.lengthSq()===0)en.z=1;if(en.normalize(),kn.crossVectors(n,en),kn.lengthSq()===0){if(Math.abs(n.z)===1)en.x+=0.0001;else en.z+=0.0001;en.normalize(),kn.crossVectors(n,en)}return kn.normalize(),Ts.crossVectors(en,kn),i[0]=kn.x,i[4]=Ts.x,i[8]=en.x,i[1]=kn.y,i[5]=Ts.y,i[9]=en.y,i[2]=kn.z,i[6]=Ts.z,i[10]=en.z,this}multiply(t){return this.multiplyMatrices(this,t)}premultiply(t){return this.multiplyMatrices(t,this)}multiplyMatrices(t,e){let n=t.elements,i=e.elements,s=this.elements,r=n[0],a=n[4],o=n[8],l=n[12],c=n[1],p=n[5],h=n[9],u=n[13],d=n[2],v=n[6],w=n[10],f=n[14],m=n[3],M=n[7],T=n[11],S=n[15],E=i[0],P=i[4],g=i[8],y=i[12],D=i[1],q=i[5],N=i[9],X=i[13],rt=i[2],R=i[6],j=i[10],tt=i[14],G=i[3],pt=i[7],J=i[11],st=i[15];return s[0]=r*E+a*D+o*rt+l*G,s[4]=r*P+a*q+o*R+l*pt,s[8]=r*g+a*N+o*j+l*J,s[12]=r*y+a*X+o*tt+l*st,s[1]=c*E+p*D+h*rt+u*G,s[5]=c*P+p*q+h*R+u*pt,s[9]=c*g+p*N+h*j+u*J,s[13]=c*y+p*X+h*tt+u*st,s[2]=d*E+v*D+w*rt+f*G,s[6]=d*P+v*q+w*R+f*pt,s[10]=d*g+v*N+w*j+f*J,s[14]=d*y+v*X+w*tt+f*st,s[3]=m*E+M*D+T*rt+S*G,s[7]=m*P+M*q+T*R+S*pt,s[11]=m*g+M*N+T*j+S*J,s[15]=m*y+M*X+T*tt+S*st,this}multiplyScalar(t){let e=this.elements;return e[0]*=t,e[4]*=t,e[8]*=t,e[12]*=t,e[1]*=t,e[5]*=t,e[9]*=t,e[13]*=t,e[2]*=t,e[6]*=t,e[10]*=t,e[14]*=t,e[3]*=t,e[7]*=t,e[11]*=t,e[15]*=t,this}determinant(){let t=this.elements,e=t[0],n=t[4],i=t[8],s=t[12],r=t[1],a=t[5],o=t[9],l=t[13],c=t[2],p=t[6],h=t[10],u=t[14],d=t[3],v=t[7],w=t[11],f=t[15],m=o*u-l*h,M=a*u-l*p,T=a*h-o*p,S=r*u-l*c,E=r*h-o*c,P=r*p-a*c;return e*(v*m-w*M+f*T)-n*(d*m-w*S+f*E)+i*(d*M-v*S+f*P)-s*(d*T-v*E+w*P)}determinantAffine(){let t=this.elements,e=t[0],n=t[4],i=t[8],s=t[1],r=t[5],a=t[9],o=t[2],l=t[6],c=t[10];return e*(r*c-a*l)-n*(s*c-a*o)+i*(s*l-r*o)}transpose(){let t=this.elements,e;return e=t[1],t[1]=t[4],t[4]=e,e=t[2],t[2]=t[8],t[8]=e,e=t[6],t[6]=t[9],t[9]=e,e=t[3],t[3]=t[12],t[12]=e,e=t[7],t[7]=t[13],t[13]=e,e=t[11],t[11]=t[14],t[14]=e,this}setPosition(t,e,n){let i=this.elements;if(t.isVector3)i[12]=t.x,i[13]=t.y,i[14]=t.z;else i[12]=t,i[13]=e,i[14]=n;return this}invert(){let t=this.elements,e=t[0],n=t[1],i=t[2],s=t[3],r=t[4],a=t[5],o=t[6],l=t[7],c=t[8],p=t[9],h=t[10],u=t[11],d=t[12],v=t[13],w=t[14],f=t[15],m=e*a-n*r,M=e*o-i*r,T=e*l-s*r,S=n*o-i*a,E=n*l-s*a,P=i*l-s*o,g=c*v-p*d,y=c*w-h*d,D=c*f-u*d,q=p*w-h*v,N=p*f-u*v,X=h*f-u*w,rt=m*X-M*N+T*q+S*D-E*y+P*g;if(rt===0)return this.set(0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0);let R=1/rt;return t[0]=(a*X-o*N+l*q)*R,t[1]=(i*N-n*X-s*q)*R,t[2]=(v*P-w*E+f*S)*R,t[3]=(h*E-p*P-u*S)*R,t[4]=(o*D-r*X-l*y)*R,t[5]=(e*X-i*D+s*y)*R,t[6]=(w*T-d*P-f*M)*R,t[7]=(c*P-h*T+u*M)*R,t[8]=(r*N-a*D+l*g)*R,t[9]=(n*D-e*N-s*g)*R,t[10]=(d*E-v*T+f*m)*R,t[11]=(p*T-c*E-u*m)*R,t[12]=(a*y-r*q-o*g)*R,t[13]=(e*q-n*y+i*g)*R,t[14]=(v*M-d*S-w*m)*R,t[15]=(c*S-p*M+h*m)*R,this}scale(t){let e=this.elements,{x:n,y:i,z:s}=t;return e[0]*=n,e[4]*=i,e[8]*=s,e[1]*=n,e[5]*=i,e[9]*=s,e[2]*=n,e[6]*=i,e[10]*=s,e[3]*=n,e[7]*=i,e[11]*=s,this}getMaxScaleOnAxis(){let t=this.elements,e=t[0]*t[0]+t[1]*t[1]+t[2]*t[2],n=t[4]*t[4]+t[5]*t[5]+t[6]*t[6],i=t[8]*t[8]+t[9]*t[9]+t[10]*t[10];return Math.sqrt(Math.max(e,n,i))}makeTranslation(t,e,n){if(t.isVector3)this.set(1,0,0,t.x,0,1,0,t.y,0,0,1,t.z,0,0,0,1);else this.set(1,0,0,t,0,1,0,e,0,0,1,n,0,0,0,1);return this}makeRotationX(t){let e=Math.cos(t),n=Math.sin(t);return this.set(1,0,0,0,0,e,-n,0,0,n,e,0,0,0,0,1),this}makeRotationY(t){let e=Math.cos(t),n=Math.sin(t);return this.set(e,0,n,0,0,1,0,0,-n,0,e,0,0,0,0,1),this}makeRotationZ(t){let e=Math.cos(t),n=Math.sin(t);return this.set(e,-n,0,0,n,e,0,0,0,0,1,0,0,0,0,1),this}makeRotationAxis(t,e){let n=Math.cos(e),i=Math.sin(e),s=1-n,{x:r,y:a,z:o}=t,l=s*r,c=s*a;return this.set(l*r+n,l*a-i*o,l*o+i*a,0,l*a+i*o,c*a+n,c*o-i*r,0,l*o-i*a,c*o+i*r,s*o*o+n,0,0,0,0,1),this}makeScale(t,e,n){return this.set(t,0,0,0,0,e,0,0,0,0,n,0,0,0,0,1),this}makeShear(t,e,n,i,s,r){return this.set(1,n,s,0,t,1,r,0,e,i,1,0,0,0,0,1),this}compose(t,e,n){let i=this.elements,{_x:s,_y:r,_z:a,_w:o}=e,l=s+s,c=r+r,p=a+a,h=s*l,u=s*c,d=s*p,v=r*c,w=r*p,f=a*p,m=o*l,M=o*c,T=o*p,{x:S,y:E,z:P}=n;return i[0]=(1-(v+f))*S,i[1]=(u+T)*S,i[2]=(d-M)*S,i[3]=0,i[4]=(u-T)*E,i[5]=(1-(h+f))*E,i[6]=(w+m)*E,i[7]=0,i[8]=(d+M)*P,i[9]=(w-m)*P,i[10]=(1-(h+v))*P,i[11]=0,i[12]=t.x,i[13]=t.y,i[14]=t.z,i[15]=1,this}decompose(t,e,n){let i=this.elements;t.x=i[12],t.y=i[13],t.z=i[14];let s=this.determinantAffine();if(s===0)return n.set(1,1,1),e.identity(),this;let r=Mi.set(i[0],i[1],i[2]).length(),a=Mi.set(i[4],i[5],i[6]).length(),o=Mi.set(i[8],i[9],i[10]).length();if(s<0)r=-r;mn.copy(this);let l=1/r,c=1/a,p=1/o;return mn.elements[0]*=l,mn.elements[1]*=l,mn.elements[2]*=l,mn.elements[4]*=c,mn.elements[5]*=c,mn.elements[6]*=c,mn.elements[8]*=p,mn.elements[9]*=p,mn.elements[10]*=p,e.setFromRotationMatrix(mn),n.x=r,n.y=a,n.z=o,this}makePerspective(t,e,n,i,s,r,a=2000,o=!1){let l=this.elements,c=2*s/(e-t),p=2*s/(n-i),h=(e+t)/(e-t),u=(n+i)/(n-i),d,v;if(o)d=s/(r-s),v=r*s/(r-s);else if(a===2000)d=-(r+s)/(r-s),v=-2*r*s/(r-s);else if(a===2001)d=-r/(r-s),v=-r*s/(r-s);else throw Error("THREE.Matrix4.makePerspective(): Invalid coordinate system: "+a);return l[0]=c,l[4]=0,l[8]=h,l[12]=0,l[1]=0,l[5]=p,l[9]=u,l[13]=0,l[2]=0,l[6]=0,l[10]=d,l[14]=v,l[3]=0,l[7]=0,l[11]=-1,l[15]=0,this}makeOrthographic(t,e,n,i,s,r,a=2000,o=!1){let l=this.elements,c=2/(e-t),p=2/(n-i),h=-(e+t)/(e-t),u=-(n+i)/(n-i),d,v;if(o)d=1/(r-s),v=r/(r-s);else if(a===2000)d=-2/(r-s),v=-(r+s)/(r-s);else if(a===2001)d=-1/(r-s),v=-s/(r-s);else throw Error("THREE.Matrix4.makeOrthographic(): Invalid coordinate system: "+a);return l[0]=c,l[4]=0,l[8]=0,l[12]=h,l[1]=0,l[5]=p,l[9]=0,l[13]=u,l[2]=0,l[6]=0,l[10]=d,l[14]=v,l[3]=0,l[7]=0,l[11]=0,l[15]=1,this}equals(t){let e=this.elements,n=t.elements;for(let i=0;i<16;i++)if(e[i]!==n[i])return!1;return!0}fromArray(t,e=0){for(let n=0;n<16;n++)this.elements[n]=t[n+e];return this}toArray(t=[],e=0){let n=this.elements;return t[e]=n[0],t[e+1]=n[1],t[e+2]=n[2],t[e+3]=n[3],t[e+4]=n[4],t[e+5]=n[5],t[e+6]=n[6],t[e+7]=n[7],t[e+8]=n[8],t[e+9]=n[9],t[e+10]=n[10],t[e+11]=n[11],t[e+12]=n[12],t[e+13]=n[13],t[e+14]=n[14],t[e+15]=n[15],t}}var Mi=new W,mn=new Se,Nh=new W(0,0,0),Dh=new W(1,1,1),kn=new W,Ts=new W,en=new W,Qo=new Se,jo=new Bn;class Xn{constructor(t=0,e=0,n=0,i=Xn.DEFAULT_ORDER){this.isEuler=!0,this._x=t,this._y=e,this._z=n,this._order=i}get x(){return this._x}set x(t){this._x=t,this._onChangeCallback()}get y(){return this._y}set y(t){this._y=t,this._onChangeCallback()}get z(){return this._z}set z(t){this._z=t,this._onChangeCallback()}get order(){return this._order}set order(t){this._order=t,this._onChangeCallback()}set(t,e,n,i=this._order){return this._x=t,this._y=e,this._z=n,this._order=i,this._onChangeCallback(),this}clone(){return new this.constructor(this._x,this._y,this._z,this._order)}copy(t){return this._x=t._x,this._y=t._y,this._z=t._z,this._order=t._order,this._onChangeCallback(),this}setFromRotationMatrix(t,e=this._order,n=!0){let i=t.elements,s=i[0],r=i[4],a=i[8],o=i[1],l=i[5],c=i[9],p=i[2],h=i[6],u=i[10];switch(e){case"XYZ":if(this._y=Math.asin(oe(a,-1,1)),Math.abs(a)<0.9999999)this._x=Math.atan2(-c,u),this._z=Math.atan2(-r,s);else this._x=Math.atan2(h,l),this._z=0;break;case"YXZ":if(this._x=Math.asin(-oe(c,-1,1)),Math.abs(c)<0.9999999)this._y=Math.atan2(a,u),this._z=Math.atan2(o,l);else this._y=Math.atan2(-p,s),this._z=0;break;case"ZXY":if(this._x=Math.asin(oe(h,-1,1)),Math.abs(h)<0.9999999)this._y=Math.atan2(-p,u),this._z=Math.atan2(-r,l);else this._y=0,this._z=Math.atan2(o,s);break;case"ZYX":if(this._y=Math.asin(-oe(p,-1,1)),Math.abs(p)<0.9999999)this._x=Math.atan2(h,u),this._z=Math.atan2(o,s);else this._x=0,this._z=Math.atan2(-r,l);break;case"YZX":if(this._z=Math.asin(oe(o,-1,1)),Math.abs(o)<0.9999999)this._x=Math.atan2(-c,l),this._y=Math.atan2(-p,s);else this._x=0,this._y=Math.atan2(a,u);break;case"XZY":if(this._z=Math.asin(-oe(r,-1,1)),Math.abs(r)<0.9999999)this._x=Math.atan2(h,l),this._y=Math.atan2(a,s);else this._x=Math.atan2(-c,u),this._y=0;break;default:Wt("Euler: .setFromRotationMatrix() encountered an unknown order: "+e)}if(this._order=e,n===!0)this._onChangeCallback();return this}setFromQuaternion(t,e,n){return Qo.makeRotationFromQuaternion(t),this.setFromRotationMatrix(Qo,e,n)}setFromVector3(t,e=this._order){return this.set(t.x,t.y,t.z,e)}reorder(t){return jo.setFromEuler(this),this.setFromQuaternion(jo,t)}equals(t){return t._x===this._x&&t._y===this._y&&t._z===this._z&&t._order===this._order}fromArray(t){if(this._x=t[0],this._y=t[1],this._z=t[2],t[3]!==void 0)this._order=t[3];return this._onChangeCallback(),this}toArray(t=[],e=0){return t[e]=this._x,t[e+1]=this._y,t[e+2]=this._z,t[e+3]=this._order,t}_onChange(t){return this._onChangeCallback=t,this}_onChangeCallback(){}*[Symbol.iterator](){yield this._x,yield this._y,yield this._z,yield this._order}}Xn.DEFAULT_ORDER="XYZ";class hr{constructor(){this.mask=1}set(t){this.mask=(1<<t|0)>>>0}enable(t){this.mask|=1<<t|0}enableAll(){this.mask=-1}toggle(t){this.mask^=1<<t|0}disable(t){this.mask&=~(1<<t|0)}disableAll(){this.mask=0}test(t){return(this.mask&t.mask)!==0}isEnabled(t){return(this.mask&(1<<t|0))!==0}}var Uh=0,tl=new W,Si=new Bn,Cn=new Se,Es=new W,$i=new W,Fh=new W,Oh=new Bn,el=new W(1,0,0),nl=new W(0,1,0),il=new W(0,0,1),sl={type:"added"},Bh={type:"removed"},bi={type:"childadded",child:null},Hr={type:"childremoved",child:null};class Xe extends On{constructor(){super();this.isObject3D=!0,Object.defineProperty(this,"id",{value:Uh++}),this.uuid=ls(),this.name="",this.type="Object3D",this.parent=null,this.children=[],this.up=Xe.DEFAULT_UP.clone();let t=new W,e=new Xn,n=new Bn,i=new W(1,1,1);function s(){n.setFromEuler(e,!1)}function r(){e.setFromQuaternion(n,void 0,!1)}e._onChange(s),n._onChange(r),Object.defineProperties(this,{position:{configurable:!0,enumerable:!0,value:t},rotation:{configurable:!0,enumerable:!0,value:e},quaternion:{configurable:!0,enumerable:!0,value:n},scale:{configurable:!0,enumerable:!0,value:i},modelViewMatrix:{value:new Se},normalMatrix:{value:new Zt}}),this.matrix=new Se,this.matrixWorld=new Se,this.matrixAutoUpdate=Xe.DEFAULT_MATRIX_AUTO_UPDATE,this.matrixWorldAutoUpdate=Xe.DEFAULT_MATRIX_WORLD_AUTO_UPDATE,this.matrixWorldNeedsUpdate=!1,this.layers=new hr,this.visible=!0,this.castShadow=!1,this.receiveShadow=!1,this.frustumCulled=!0,this.renderOrder=0,this.animations=[],this.customDepthMaterial=void 0,this.customDistanceMaterial=void 0,this.static=!1,this.userData={},this.pivot=null}onBeforeShadow(){}onAfterShadow(){}onBeforeRender(){}onAfterRender(){}applyMatrix4(t){if(this.matrixAutoUpdate)this.updateMatrix();if(this.matrix.premultiply(t),this.matrix.decompose(this.position,this.quaternion,this.scale),this.pivot!==null)this.position.copy(this.pivot).applyMatrix4(this.matrix).sub(this.pivot)}applyQuaternion(t){return this.quaternion.premultiply(t),this}setRotationFromAxisAngle(t,e){this.quaternion.setFromAxisAngle(t,e)}setRotationFromEuler(t){this.quaternion.setFromEuler(t,!0)}setRotationFromMatrix(t){this.quaternion.setFromRotationMatrix(t)}setRotationFromQuaternion(t){this.quaternion.copy(t)}rotateOnAxis(t,e){return Si.setFromAxisAngle(t,e),this.quaternion.multiply(Si),this}rotateOnWorldAxis(t,e){return Si.setFromAxisAngle(t,e),this.quaternion.premultiply(Si),this}rotateX(t){return this.rotateOnAxis(el,t)}rotateY(t){return this.rotateOnAxis(nl,t)}rotateZ(t){return this.rotateOnAxis(il,t)}translateOnAxis(t,e){return tl.copy(t).applyQuaternion(this.quaternion),this.position.add(tl.multiplyScalar(e)),this}translateX(t){return this.translateOnAxis(el,t)}translateY(t){return this.translateOnAxis(nl,t)}translateZ(t){return this.translateOnAxis(il,t)}localToWorld(t){return this.updateWorldMatrix(!0,!1),t.applyMatrix4(this.matrixWorld)}worldToLocal(t){return this.updateWorldMatrix(!0,!1),t.applyMatrix4(Cn.copy(this.matrixWorld).invert())}lookAt(t,e,n){if(t.isVector3)Es.copy(t);else Es.set(t,e,n);let i=this.parent;if(this.updateWorldMatrix(!0,!1),$i.setFromMatrixPosition(this.matrixWorld),this.isCamera||this.isLight)Cn.lookAt($i,Es,this.up);else Cn.lookAt(Es,$i,this.up);if(this.quaternion.setFromRotationMatrix(Cn),i)Cn.extractRotation(i.matrixWorld),Si.setFromRotationMatrix(Cn),this.quaternion.premultiply(Si.invert())}add(t){if(arguments.length>1){for(let e=0;e<arguments.length;e++)this.add(arguments[e]);return this}if(t===this)return Yt("Object3D.add: object can't be added as a child of itself.",t),this;if(t&&t.isObject3D)t.removeFromParent(),t.parent=this,this.children.push(t),t.dispatchEvent(sl),bi.child=t,this.dispatchEvent(bi),bi.child=null;else Yt("Object3D.add: object not an instance of THREE.Object3D.",t);return this}remove(t){if(arguments.length>1){for(let n=0;n<arguments.length;n++)this.remove(arguments[n]);return this}let e=this.children.indexOf(t);if(e!==-1)t.parent=null,this.children.splice(e,1),t.dispatchEvent(Bh),Hr.child=t,this.dispatchEvent(Hr),Hr.child=null;return this}removeFromParent(){let t=this.parent;if(t!==null)t.remove(this);return this}clear(){return this.remove(...this.children)}attach(t){if(this.updateWorldMatrix(!0,!1),Cn.copy(this.matrixWorld).invert(),t.parent!==null)t.parent.updateWorldMatrix(!0,!1),Cn.multiply(t.parent.matrixWorld);return t.applyMatrix4(Cn),t.removeFromParent(),t.parent=this,this.children.push(t),t.updateWorldMatrix(!1,!0),t.dispatchEvent(sl),bi.child=t,this.dispatchEvent(bi),bi.child=null,this}getObjectById(t){return this.getObjectByProperty("id",t)}getObjectByName(t){return this.getObjectByProperty("name",t)}getObjectByProperty(t,e){if(this[t]===e)return this;for(let n=0,i=this.children.length;n<i;n++){let r=this.children[n].getObjectByProperty(t,e);if(r!==void 0)return r}return}getObjectsByProperty(t,e,n=[]){if(this[t]===e)n.push(this);let i=this.children;for(let s=0,r=i.length;s<r;s++)i[s].getObjectsByProperty(t,e,n);return n}getWorldPosition(t){return this.updateWorldMatrix(!0,!1),t.setFromMatrixPosition(this.matrixWorld)}getWorldQuaternion(t){return this.updateWorldMatrix(!0,!1),this.matrixWorld.decompose($i,t,Fh),t}getWorldScale(t){return this.updateWorldMatrix(!0,!1),this.matrixWorld.decompose($i,Oh,t),t}getWorldDirection(t){this.updateWorldMatrix(!0,!1);let e=this.matrixWorld.elements;return t.set(e[8],e[9],e[10]).normalize()}raycast(){}intersectsFrustum(){return!0}traverse(t){t(this);let e=this.children;for(let n=0,i=e.length;n<i;n++)e[n].traverse(t)}traverseVisible(t){if(this.visible===!1)return;t(this);let e=this.children;for(let n=0,i=e.length;n<i;n++)e[n].traverseVisible(t)}traverseAncestors(t){let e=this.parent;if(e!==null)t(e),e.traverseAncestors(t)}updateMatrix(){this.matrix.compose(this.position,this.quaternion,this.scale);let t=this.pivot;if(t!==null){let{x:e,y:n,z:i}=t,s=this.matrix.elements;s[12]+=e-s[0]*e-s[4]*n-s[8]*i,s[13]+=n-s[1]*e-s[5]*n-s[9]*i,s[14]+=i-s[2]*e-s[6]*n-s[10]*i}this.matrixWorldNeedsUpdate=!0}updateMatrixWorld(t){if(this.matrixAutoUpdate)this.updateMatrix();if(this.matrixWorldNeedsUpdate||t){if(this.matrixWorldAutoUpdate===!0)if(this.parent===null)this.matrixWorld.copy(this.matrix);else this.matrixWorld.multiplyMatrices(this.parent.matrixWorld,this.matrix);this.matrixWorldNeedsUpdate=!1,t=!0}let e=this.children;for(let n=0,i=e.length;n<i;n++)e[n].updateMatrixWorld(t)}updateWorldMatrix(t,e,n=!1){let i=this.parent;if(t===!0&&i!==null)i.updateWorldMatrix(!0,!1);if(this.matrixAutoUpdate)this.updateMatrix();if(this.matrixWorldNeedsUpdate||n){if(this.matrixWorldAutoUpdate===!0)if(this.parent===null)this.matrixWorld.copy(this.matrix);else this.matrixWorld.multiplyMatrices(this.parent.matrixWorld,this.matrix);this.matrixWorldNeedsUpdate=!1,n=!0}if(e===!0){let s=this.children;for(let r=0,a=s.length;r<a;r++)s[r].updateWorldMatrix(!1,!0,n)}}toJSON(t){let e=t===void 0||typeof t==="string",n={};if(e)t={geometries:{},materials:{},textures:{},images:{},shapes:{},skeletons:{},animations:{},nodes:{}},n.metadata={version:4.7,type:"Object",generator:"Object3D.toJSON"};let i={};if(i.uuid=this.uuid,i.type=this.type,i.name=this.name,i.castShadow=this.castShadow,i.receiveShadow=this.receiveShadow,i.visible=this.visible,i.frustumCulled=this.frustumCulled,i.renderOrder=this.renderOrder,i.static=this.static,i.matrixAutoUpdate=this.matrixAutoUpdate,Object.keys(this.userData).length>0)i.userData=this.userData;if(i.layers=this.layers.mask,i.matrix=this.matrix.toArray(),i.up=this.up.toArray(),this.pivot!==null)i.pivot=this.pivot.toArray();if(this.morphTargetDictionary!==void 0)i.morphTargetDictionary=Object.assign({},this.morphTargetDictionary);if(this.morphTargetInfluences!==void 0)i.morphTargetInfluences=this.morphTargetInfluences.slice();if(this.isInstancedMesh){if(i.type="InstancedMesh",i.count=this.count,i.instanceMatrix=this.instanceMatrix.toJSON(),this.instanceColor!==null)i.instanceColor=this.instanceColor.toJSON()}if(this.isBatchedMesh){if(i.type="BatchedMesh",i.perObjectFrustumCulled=this.perObjectFrustumCulled,i.sortObjects=this.sortObjects,i.drawRanges=this._drawRanges,i.reservedRanges=this._reservedRanges,i.geometryInfo=this._geometryInfo.map((a)=>({...a,boundingBox:a.boundingBox?a.boundingBox.toJSON():void 0,boundingSphere:a.boundingSphere?a.boundingSphere.toJSON():void 0})),i.instanceInfo=this._instanceInfo.map((a)=>({...a})),i.availableInstanceIds=this._availableInstanceIds.slice(),i.availableGeometryIds=this._availableGeometryIds.slice(),i.nextIndexStart=this._nextIndexStart,i.nextVertexStart=this._nextVertexStart,i.geometryCount=this._geometryCount,i.maxInstanceCount=this._maxInstanceCount,i.maxVertexCount=this._maxVertexCount,i.maxIndexCount=this._maxIndexCount,i.geometryInitialized=this._geometryInitialized,i.matricesTexture=this._matricesTexture.toJSON(t),i.indirectTexture=this._indirectTexture.toJSON(t),this._colorsTexture!==null)i.colorsTexture=this._colorsTexture.toJSON(t);if(this.boundingSphere!==null)i.boundingSphere=this.boundingSphere.toJSON();if(this.boundingBox!==null)i.boundingBox=this.boundingBox.toJSON()}function s(a,o){if(a[o.uuid]===void 0)a[o.uuid]=o.toJSON(t);return o.uuid}if(this.isScene){if(this.background){if(this.background.isColor)i.background=this.background.toJSON();else if(this.background.isTexture)i.background=this.background.toJSON(t).uuid}if(this.environment&&this.environment.isTexture&&this.environment.isRenderTargetTexture!==!0)i.environment=this.environment.toJSON(t).uuid}else if(this.isMesh||this.isLine||this.isPoints){i.geometry=s(t.geometries,this.geometry);let a=this.geometry.parameters;if(a!==void 0&&a.shapes!==void 0){let o=a.shapes;if(Array.isArray(o))for(let l=0,c=o.length;l<c;l++){let p=o[l];s(t.shapes,p)}else s(t.shapes,o)}}if(this.isSkinnedMesh){if(i.bindMode=this.bindMode,i.bindMatrix=this.bindMatrix.toArray(),this.skeleton!==void 0)s(t.skeletons,this.skeleton),i.skeleton=this.skeleton.uuid}if(this.material!==void 0)if(Array.isArray(this.material)){let a=[];for(let o=0,l=this.material.length;o<l;o++)a.push(s(t.materials,this.material[o]));i.material=a}else i.material=s(t.materials,this.material);if(this.children.length>0){i.children=[];for(let a=0;a<this.children.length;a++)i.children.push(this.children[a].toJSON(t).object)}if(this.animations.length>0){i.animations=[];for(let a=0;a<this.animations.length;a++){let o=this.animations[a];i.animations.push(s(t.animations,o))}}if(e){let a=r(t.geometries),o=r(t.materials),l=r(t.textures),c=r(t.images),p=r(t.shapes),h=r(t.skeletons),u=r(t.animations),d=r(t.nodes);if(a.length>0)n.geometries=a;if(o.length>0)n.materials=o;if(l.length>0)n.textures=l;if(c.length>0)n.images=c;if(p.length>0)n.shapes=p;if(h.length>0)n.skeletons=h;if(u.length>0)n.animations=u;if(d.length>0)n.nodes=d}return n.object=i,n;function r(a){let o=[];for(let l in a){let c=a[l];delete c.metadata,o.push(c)}return o}}clone(t){return new this.constructor().copy(this,t)}copy(t,e=!0){if(this.name=t.name,this.up.copy(t.up),this.position.copy(t.position),this.rotation.order=t.rotation.order,this.quaternion.copy(t.quaternion),this.scale.copy(t.scale),this.pivot=t.pivot!==null?t.pivot.clone():null,this.matrix.copy(t.matrix),this.matrixWorld.copy(t.matrixWorld),this.matrixAutoUpdate=t.matrixAutoUpdate,this.matrixWorldAutoUpdate=t.matrixWorldAutoUpdate,this.matrixWorldNeedsUpdate=t.matrixWorldNeedsUpdate,this.layers.mask=t.layers.mask,this.visible=t.visible,this.castShadow=t.castShadow,this.receiveShadow=t.receiveShadow,this.frustumCulled=t.frustumCulled,this.renderOrder=t.renderOrder,this.static=t.static,this.animations=t.animations.slice(),this.userData=JSON.parse(JSON.stringify(t.userData)),e===!0)for(let n=0;n<t.children.length;n++){let i=t.children[n];this.add(i.clone())}return this}dispose(){this.dispatchEvent({type:"dispose"})}}Xe.DEFAULT_UP=new W(0,1,0);Xe.DEFAULT_MATRIX_AUTO_UPDATE=!0;Xe.DEFAULT_MATRIX_WORLD_AUTO_UPDATE=!0;class Ni extends Xe{constructor(){super();this.isGroup=!0,this.type="Group"}}var zh={type:"move"};class hs{constructor(){this._targetRay=null,this._grip=null,this._hand=null}getHandSpace(){if(this._hand===null)this._hand=new Ni,this._hand.matrixAutoUpdate=!1,this._hand.visible=!1,this._hand.joints={},this._hand.inputState={pinching:!1};return this._hand}getTargetRaySpace(){if(this._targetRay===null)this._targetRay=new Ni,this._targetRay.matrixAutoUpdate=!1,this._targetRay.visible=!1,this._targetRay.hasLinearVelocity=!1,this._targetRay.linearVelocity=new W,this._targetRay.hasAngularVelocity=!1,this._targetRay.angularVelocity=new W;return this._targetRay}getGripSpace(){if(this._grip===null)this._grip=new Ni,this._grip.matrixAutoUpdate=!1,this._grip.visible=!1,this._grip.hasLinearVelocity=!1,this._grip.linearVelocity=new W,this._grip.hasAngularVelocity=!1,this._grip.angularVelocity=new W,this._grip.eventsEnabled=!1;return this._grip}dispatchEvent(t){if(this._targetRay!==null)this._targetRay.dispatchEvent(t);if(this._grip!==null)this._grip.dispatchEvent(t);if(this._hand!==null)this._hand.dispatchEvent(t);return this}connect(t){if(t&&t.hand){let e=this._hand;if(e)for(let n of t.hand.values())this._getHandJoint(e,n)}return this.dispatchEvent({type:"connected",data:t}),this}disconnect(t){if(this.dispatchEvent({type:"disconnected",data:t}),this._targetRay!==null)this._targetRay.visible=!1;if(this._grip!==null)this._grip.visible=!1;if(this._hand!==null)this._hand.visible=!1;return this}update(t,e,n){let i=null,s=null,r=null,a=this._targetRay,o=this._grip,l=this._hand;if(t&&e.session.visibilityState!=="visible-blurred"){if(l&&t.hand){r=!0;for(let v of t.hand.values()){let w=e.getJointPose(v,n),f=this._getHandJoint(l,v);if(w!==null)f.matrix.fromArray(w.transform.matrix),f.matrix.decompose(f.position,f.quaternion,f.scale),f.matrixWorldNeedsUpdate=!0,f.jointRadius=w.radius;f.visible=w!==null}let c=l.joints["index-finger-tip"],p=l.joints["thumb-tip"],h=c.position.distanceTo(p.position),u=0.02,d=0.005;if(l.inputState.pinching&&h>u+d)l.inputState.pinching=!1,this.dispatchEvent({type:"pinchend",handedness:t.handedness,target:this});else if(!l.inputState.pinching&&h<=u-d)l.inputState.pinching=!0,this.dispatchEvent({type:"pinchstart",handedness:t.handedness,target:this})}else if(o!==null&&t.gripSpace){if(s=e.getPose(t.gripSpace,n),s!==null){if(o.matrix.fromArray(s.transform.matrix),o.matrix.decompose(o.position,o.quaternion,o.scale),o.matrixWorldNeedsUpdate=!0,s.linearVelocity)o.hasLinearVelocity=!0,o.linearVelocity.copy(s.linearVelocity);else o.hasLinearVelocity=!1;if(s.angularVelocity)o.hasAngularVelocity=!0,o.angularVelocity.copy(s.angularVelocity);else o.hasAngularVelocity=!1;if(o.eventsEnabled)o.dispatchEvent({type:"gripUpdated",data:t,target:this})}}if(a!==null){if(i=e.getPose(t.targetRaySpace,n),i===null&&s!==null)i=s;if(i!==null){if(a.matrix.fromArray(i.transform.matrix),a.matrix.decompose(a.position,a.quaternion,a.scale),a.matrixWorldNeedsUpdate=!0,i.linearVelocity)a.hasLinearVelocity=!0,a.linearVelocity.copy(i.linearVelocity);else a.hasLinearVelocity=!1;if(i.angularVelocity)a.hasAngularVelocity=!0,a.angularVelocity.copy(i.angularVelocity);else a.hasAngularVelocity=!1;this.dispatchEvent(zh)}}}if(a!==null)a.visible=i!==null;if(o!==null)o.visible=s!==null;if(l!==null)l.visible=r!==null;return this}_getHandJoint(t,e){if(t.joints[e.jointName]===void 0){let n=new Ni;n.matrixAutoUpdate=!1,n.visible=!1,t.joints[e.jointName]=n,t.add(n)}return t.joints[e.jointName]}}var bc={aliceblue:15792383,antiquewhite:16444375,aqua:65535,aquamarine:8388564,azure:15794175,beige:16119260,bisque:16770244,black:0,blanchedalmond:16772045,blue:255,blueviolet:9055202,brown:10824234,burlywood:14596231,cadetblue:6266528,chartreuse:8388352,chocolate:13789470,coral:16744272,cornflowerblue:6591981,cornsilk:16775388,crimson:14423100,cyan:65535,darkblue:139,darkcyan:35723,darkgoldenrod:12092939,darkgray:11119017,darkgreen:25600,darkgrey:11119017,darkkhaki:12433259,darkmagenta:9109643,darkolivegreen:5597999,darkorange:16747520,darkorchid:10040012,darkred:9109504,darksalmon:15308410,darkseagreen:9419919,darkslateblue:4734347,darkslategray:3100495,darkslategrey:3100495,darkturquoise:52945,darkviolet:9699539,deeppink:16716947,deepskyblue:49151,dimgray:6908265,dimgrey:6908265,dodgerblue:2003199,firebrick:11674146,floralwhite:16775920,forestgreen:2263842,fuchsia:16711935,gainsboro:14474460,ghostwhite:16316671,gold:16766720,goldenrod:14329120,gray:8421504,green:32768,greenyellow:11403055,grey:8421504,honeydew:15794160,hotpink:16738740,indianred:13458524,indigo:4915330,ivory:16777200,khaki:15787660,lavender:15132410,lavenderblush:16773365,lawngreen:8190976,lemonchiffon:16775885,lightblue:11393254,lightcoral:15761536,lightcyan:14745599,lightgoldenrodyellow:16448210,lightgray:13882323,lightgreen:9498256,lightgrey:13882323,lightpink:16758465,lightsalmon:16752762,lightseagreen:2142890,lightskyblue:8900346,lightslategray:7833753,lightslategrey:7833753,lightsteelblue:11584734,lightyellow:16777184,lime:65280,limegreen:3329330,linen:16445670,magenta:16711935,maroon:8388608,mediumaquamarine:6737322,mediumblue:205,mediumorchid:12211667,mediumpurple:9662683,mediumseagreen:3978097,mediumslateblue:8087790,mediumspringgreen:64154,mediumturquoise:4772300,mediumvioletred:13047173,midnightblue:1644912,mintcream:16121850,mistyrose:16770273,moccasin:16770229,navajowhite:16768685,navy:128,oldlace:16643558,olive:8421376,olivedrab:7048739,orange:16753920,orangered:16729344,orchid:14315734,palegoldenrod:15657130,palegreen:10025880,paleturquoise:11529966,palevioletred:14381203,papayawhip:16773077,peachpuff:16767673,peru:13468991,pink:16761035,plum:14524637,powderblue:11591910,purple:8388736,rebeccapurple:6697881,red:16711680,rosybrown:12357519,royalblue:4286945,saddlebrown:9127187,salmon:16416882,sandybrown:16032864,seagreen:3050327,seashell:16774638,sienna:10506797,silver:12632256,skyblue:8900331,slateblue:6970061,slategray:7372944,slategrey:7372944,snow:16775930,springgreen:65407,steelblue:4620980,tan:13808780,teal:32896,thistle:14204888,tomato:16737095,turquoise:4251856,violet:15631086,wheat:16113331,white:16777215,whitesmoke:16119285,yellow:16776960,yellowgreen:10145074},Gn={h:0,s:0,l:0},Rs={h:0,s:0,l:0};function Wr(t,e,n){if(n<0)n+=1;if(n>1)n-=1;if(n<0.16666666666666666)return t+(e-t)*6*n;if(n<0.5)return e;if(n<0.6666666666666666)return t+(e-t)*6*(0.6666666666666666-n);return t}class le{constructor(t,e,n){return this.isColor=!0,this.r=1,this.g=1,this.b=1,this.set(t,e,n)}set(t,e,n){if(e===void 0&&n===void 0){let i=t;if(i&&i.isColor)this.copy(i);else if(typeof i==="number")this.setHex(i);else if(typeof i==="string")this.setStyle(i)}else this.setRGB(t,e,n);return this}setScalar(t){return this.r=t,this.g=t,this.b=t,this}setHex(t,e="srgb"){return t=Math.floor(t),this.r=(t>>16&255)/255,this.g=(t>>8&255)/255,this.b=(t&255)/255,ae.colorSpaceToWorking(this,e),this}setRGB(t,e,n,i=ae.workingColorSpace){return this.r=t,this.g=e,this.b=n,ae.colorSpaceToWorking(this,i),this}setHSL(t,e,n,i=ae.workingColorSpace){if(t=Eh(t,1),e=oe(e,0,1),n=oe(n,0,1),e===0)this.r=this.g=this.b=n;else{let s=n<=0.5?n*(1+e):n+e-n*e,r=2*n-s;this.r=Wr(r,s,t+0.3333333333333333),this.g=Wr(r,s,t),this.b=Wr(r,s,t-0.3333333333333333)}return ae.colorSpaceToWorking(this,i),this}setStyle(t,e="srgb"){function n(s){if(s===void 0)return;if(parseFloat(s)<1)Wt("Color: Alpha component of "+t+" will be ignored.")}let i;if(i=/^(\w+)\(([^\)]*)\)/.exec(t)){let s,r=i[1],a=i[2];switch(r){case"rgb":case"rgba":if(s=/^\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*(?:,\s*(\d+(?:\.\d+)?|\.\d+)\s*)?$/.exec(a))return n(s[4]),this.setRGB(Math.min(255,parseInt(s[1],10))/255,Math.min(255,parseInt(s[2],10))/255,Math.min(255,parseInt(s[3],10))/255,e);if(s=/^\s*(\d+(?:\.\d+)?|\.\d+)\%\s*,\s*(\d+(?:\.\d+)?|\.\d+)\%\s*,\s*(\d+(?:\.\d+)?|\.\d+)\%\s*(?:,\s*(\d+(?:\.\d+)?|\.\d+)\s*)?$/.exec(a))return n(s[4]),this.setRGB(Math.min(100,parseFloat(s[1]))/100,Math.min(100,parseFloat(s[2]))/100,Math.min(100,parseFloat(s[3]))/100,e);break;case"hsl":case"hsla":if(s=/^\s*([+-]?(?:\d+(?:\.\d+)?|\.\d+))\s*,\s*(\d+(?:\.\d+)?|\.\d+)\%\s*,\s*(\d+(?:\.\d+)?|\.\d+)\%\s*(?:,\s*(\d+(?:\.\d+)?|\.\d+)\s*)?$/.exec(a))return n(s[4]),this.setHSL(parseFloat(s[1])/360,parseFloat(s[2])/100,parseFloat(s[3])/100,e);break;default:Wt("Color: Unknown color model "+t)}}else if(i=/^\#([A-Fa-f\d]+)$/.exec(t)){let s=i[1],r=s.length;if(r===3)return this.setRGB(parseInt(s.charAt(0),16)/15,parseInt(s.charAt(1),16)/15,parseInt(s.charAt(2),16)/15,e);else if(r===6)return this.setHex(parseInt(s,16),e);else Wt("Color: Invalid hex color "+t)}else if(t&&t.length>0)return this.setColorName(t,e);return this}setColorName(t,e="srgb"){let n=bc[t.toLowerCase()];if(n!==void 0)this.setHex(n,e);else Wt("Color: Unknown color "+t);return this}clone(){return new this.constructor(this.r,this.g,this.b)}copy(t){return this.r=t.r,this.g=t.g,this.b=t.b,this}copySRGBToLinear(t){return this.r=Dn(t.r),this.g=Dn(t.g),this.b=Dn(t.b),this}copyLinearToSRGB(t){return this.r=Di(t.r),this.g=Di(t.g),this.b=Di(t.b),this}convertSRGBToLinear(){return this.copySRGBToLinear(this),this}convertLinearToSRGB(){return this.copyLinearToSRGB(this),this}getHex(t="srgb"){return ae.workingToColorSpace(We.copy(this),t),Math.round(oe(We.r*255,0,255))*65536+Math.round(oe(We.g*255,0,255))*256+Math.round(oe(We.b*255,0,255))}getHexString(t="srgb"){return("000000"+this.getHex(t).toString(16)).slice(-6)}getHSL(t,e=ae.workingColorSpace){ae.workingToColorSpace(We.copy(this),e);let{r:n,g:i,b:s}=We,r=Math.max(n,i,s),a=Math.min(n,i,s),o,l,c=(a+r)/2;if(a===r)o=0,l=0;else{let p=r-a;switch(l=c<=0.5?p/(r+a):p/(2-r-a),r){case n:o=(i-s)/p+(i<s?6:0);break;case i:o=(s-n)/p+2;break;case s:o=(n-i)/p+4;break}o/=6}return t.h=o,t.s=l,t.l=c,t}getRGB(t,e=ae.workingColorSpace){return ae.workingToColorSpace(We.copy(this),e),t.r=We.r,t.g=We.g,t.b=We.b,t}getStyle(t="srgb"){ae.workingToColorSpace(We.copy(this),t);let{r:e,g:n,b:i}=We;if(t!=="srgb")return`color(${t} ${e.toFixed(3)} ${n.toFixed(3)} ${i.toFixed(3)})`;return`rgb(${Math.round(e*255)},${Math.round(n*255)},${Math.round(i*255)})`}offsetHSL(t,e,n){return this.getHSL(Gn),this.setHSL(Gn.h+t,Gn.s+e,Gn.l+n)}add(t){return this.r+=t.r,this.g+=t.g,this.b+=t.b,this}addColors(t,e){return this.r=t.r+e.r,this.g=t.g+e.g,this.b=t.b+e.b,this}addScalar(t){return this.r+=t,this.g+=t,this.b+=t,this}sub(t){return this.r=Math.max(0,this.r-t.r),this.g=Math.max(0,this.g-t.g),this.b=Math.max(0,this.b-t.b),this}multiply(t){return this.r*=t.r,this.g*=t.g,this.b*=t.b,this}multiplyScalar(t){return this.r*=t,this.g*=t,this.b*=t,this}lerp(t,e){return this.r+=(t.r-this.r)*e,this.g+=(t.g-this.g)*e,this.b+=(t.b-this.b)*e,this}lerpColors(t,e,n){return this.r=t.r+(e.r-t.r)*n,this.g=t.g+(e.g-t.g)*n,this.b=t.b+(e.b-t.b)*n,this}lerpHSL(t,e){this.getHSL(Gn),t.getHSL(Rs);let n=Or(Gn.h,Rs.h,e),i=Or(Gn.s,Rs.s,e),s=Or(Gn.l,Rs.l,e);return this.setHSL(n,i,s),this}setFromVector3(t){return this.r=t.x,this.g=t.y,this.b=t.z,this}applyMatrix3(t){let e=this.r,n=this.g,i=this.b,s=t.elements;return this.r=s[0]*e+s[3]*n+s[6]*i,this.g=s[1]*e+s[4]*n+s[7]*i,this.b=s[2]*e+s[5]*n+s[8]*i,this}equals(t){return t.r===this.r&&t.g===this.g&&t.b===this.b}fromArray(t,e=0){return this.r=t[e],this.g=t[e+1],this.b=t[e+2],this}toArray(t=[],e=0){return t[e]=this.r,t[e+1]=this.g,t[e+2]=this.b,t}fromBufferAttribute(t,e){return this.r=t.getX(e),this.g=t.getY(e),this.b=t.getZ(e),this}toJSON(){return this.getHex()}*[Symbol.iterator](){yield this.r,yield this.g,yield this.b}}var We=new le;le.NAMES=bc;class ur extends Xe{constructor(){super();if(this.isScene=!0,this.type="Scene",this.background=null,this.environment=null,this.fog=null,this.backgroundBlurriness=0,this.backgroundIntensity=1,this.backgroundRotation=new Xn,this.environmentIntensity=1,this.environmentRotation=new Xn,this.overrideMaterial=null,typeof __THREE_DEVTOOLS__<"u")__THREE_DEVTOOLS__.dispatchEvent(new CustomEvent("observe",{detail:this}))}copy(t,e){return super.copy(t,e),this.background=t.background!==null?t.background.clone():null,this.environment=t.environment!==null?t.environment.clone():null,this.fog=t.fog!==null?t.fog.clone():null,this.backgroundBlurriness=t.backgroundBlurriness,this.backgroundIntensity=t.backgroundIntensity,this.backgroundRotation.copy(t.backgroundRotation),this.environmentIntensity=t.environmentIntensity,this.environmentRotation.copy(t.environmentRotation),this.overrideMaterial=t.overrideMaterial!==null?t.overrideMaterial.clone():null,this.matrixAutoUpdate=t.matrixAutoUpdate,this}toJSON(t){let e=super.toJSON(t);if(this.fog!==null)e.object.fog=this.fog.toJSON();return e.object.backgroundBlurriness=this.backgroundBlurriness,e.object.backgroundIntensity=this.backgroundIntensity,e.object.backgroundRotation=this.backgroundRotation.toArray(),e.object.environmentIntensity=this.environmentIntensity,e.object.environmentRotation=this.environmentRotation.toArray(),e}}var gn=new W,In=new W,Vr=new W,Pn=new W,wi=new W,Ai=new W,rl=new W,Xr=new W,qr=new W,Yr=new W,Zr=new Ae,Jr=new Ae,$r=new Ae;class on{constructor(t=new W,e=new W,n=new W){this.a=t,this.b=e,this.c=n}static getNormal(t,e,n,i){i.subVectors(n,e),gn.subVectors(t,e),i.cross(gn);let s=i.lengthSq();if(s>0)return i.multiplyScalar(1/Math.sqrt(s));return i.set(0,0,0)}static getBarycoord(t,e,n,i,s){gn.subVectors(i,e),In.subVectors(n,e),Vr.subVectors(t,e);let r=gn.dot(gn),a=gn.dot(In),o=gn.dot(Vr),l=In.dot(In),c=In.dot(Vr),p=r*l-a*a;if(p===0)return s.set(0,0,0),null;let h=1/p,u=(l*o-a*c)*h,d=(r*c-a*o)*h;return s.set(1-u-d,d,u)}static containsPoint(t,e,n,i){if(this.getBarycoord(t,e,n,i,Pn)===null)return!1;return Pn.x>=0&&Pn.y>=0&&Pn.x+Pn.y<=1}static getInterpolation(t,e,n,i,s,r,a,o){if(this.getBarycoord(t,e,n,i,Pn)===null){if(o.x=0,o.y=0,"z"in o)o.z=0;if("w"in o)o.w=0;return null}return o.setScalar(0),o.addScaledVector(s,Pn.x),o.addScaledVector(r,Pn.y),o.addScaledVector(a,Pn.z),o}static getInterpolatedAttribute(t,e,n,i,s,r){return Zr.setScalar(0),Jr.setScalar(0),$r.setScalar(0),Zr.fromBufferAttribute(t,e),Jr.fromBufferAttribute(t,n),$r.fromBufferAttribute(t,i),r.setScalar(0),r.addScaledVector(Zr,s.x),r.addScaledVector(Jr,s.y),r.addScaledVector($r,s.z),r}static isFrontFacing(t,e,n,i){return gn.subVectors(n,e),In.subVectors(t,e),gn.cross(In).dot(i)<0}set(t,e,n){return this.a.copy(t),this.b.copy(e),this.c.copy(n),this}setFromPointsAndIndices(t,e,n,i){return this.a.copy(t[e]),this.b.copy(t[n]),this.c.copy(t[i]),this}setFromAttributeAndIndices(t,e,n,i){return this.a.fromBufferAttribute(t,e),this.b.fromBufferAttribute(t,n),this.c.fromBufferAttribute(t,i),this}clone(){return new this.constructor().copy(this)}copy(t){return this.a.copy(t.a),this.b.copy(t.b),this.c.copy(t.c),this}getArea(){return gn.subVectors(this.c,this.b),In.subVectors(this.a,this.b),gn.cross(In).length()*0.5}getMidpoint(t){return t.addVectors(this.a,this.b).add(this.c).multiplyScalar(0.3333333333333333)}getNormal(t){return on.getNormal(this.a,this.b,this.c,t)}getPlane(t){return t.setFromCoplanarPoints(this.a,this.b,this.c)}getBarycoord(t,e){return on.getBarycoord(t,this.a,this.b,this.c,e)}getInterpolation(t,e,n,i,s){return on.getInterpolation(t,this.a,this.b,this.c,e,n,i,s)}containsPoint(t){return on.containsPoint(t,this.a,this.b,this.c)}isFrontFacing(t){return on.isFrontFacing(this.a,this.b,this.c,t)}intersectsBox(t){return t.intersectsTriangle(this)}closestPointToPoint(t,e){let n=this.a,i=this.b,s=this.c,r,a;wi.subVectors(i,n),Ai.subVectors(s,n),Xr.subVectors(t,n);let o=wi.dot(Xr),l=Ai.dot(Xr);if(o<=0&&l<=0)return e.copy(n);qr.subVectors(t,i);let c=wi.dot(qr),p=Ai.dot(qr);if(c>=0&&p<=c)return e.copy(i);let h=o*p-c*l;if(h<=0&&o>=0&&c<=0&&o-c>0)return r=o/(o-c),e.copy(n).addScaledVector(wi,r);Yr.subVectors(t,s);let u=wi.dot(Yr),d=Ai.dot(Yr);if(d>=0&&u<=d)return e.copy(s);let v=u*l-o*d;if(v<=0&&l>=0&&d<=0)return a=l/(l-d),e.copy(n).addScaledVector(Ai,a);let w=c*d-u*p;if(w<=0&&p-c>=0&&u-d>=0)return rl.subVectors(s,i),a=(p-c)/(p-c+(u-d)),e.copy(i).addScaledVector(rl,a);let f=1/(w+v+h);return r=v*f,a=h*f,e.copy(n).addScaledVector(wi,r).addScaledVector(Ai,a)}equals(t){return t.a.equals(this.a)&&t.b.equals(this.b)&&t.c.equals(this.c)}}class ui{constructor(t=new W(1/0,1/0,1/0),e=new W(-1/0,-1/0,-1/0)){this.isBox3=!0,this.min=t,this.max=e}set(t,e){return this.min.copy(t),this.max.copy(e),this}setFromArray(t){this.makeEmpty();for(let e=0,n=t.length;e<n;e+=3)this.expandByPoint(_n.fromArray(t,e));return this}setFromBufferAttribute(t){this.makeEmpty();for(let e=0,n=t.count;e<n;e++)this.expandByPoint(_n.fromBufferAttribute(t,e));return this}setFromPoints(t){this.makeEmpty();for(let e=0,n=t.length;e<n;e++)this.expandByPoint(t[e]);return this}setFromCenterAndSize(t,e){let n=_n.copy(e).multiplyScalar(0.5);return this.min.copy(t).sub(n),this.max.copy(t).add(n),this}setFromObject(t,e=!1){return this.makeEmpty(),this.expandByObject(t,e)}clone(){return new this.constructor().copy(this)}copy(t){return this.min.copy(t.min),this.max.copy(t.max),this}makeEmpty(){return this.min.x=this.min.y=this.min.z=1/0,this.max.x=this.max.y=this.max.z=-1/0,this}isEmpty(){return this.max.x<this.min.x||this.max.y<this.min.y||this.max.z<this.min.z}getCenter(t){return this.isEmpty()?t.set(0,0,0):t.addVectors(this.min,this.max).multiplyScalar(0.5)}getSize(t){return this.isEmpty()?t.set(0,0,0):t.subVectors(this.max,this.min)}expandByPoint(t){return this.min.min(t),this.max.max(t),this}expandByVector(t){return this.min.sub(t),this.max.add(t),this}expandByScalar(t){return this.min.addScalar(-t),this.max.addScalar(t),this}expandByObject(t,e=!1){t.updateWorldMatrix(!1,!1);let n=t.geometry;if(n!==void 0){let s=n.getAttribute("position");if(e===!0&&s!==void 0&&t.isInstancedMesh!==!0&&t.isBatchedMesh!==!0)for(let r=0,a=s.count;r<a;r++){if(t.isMesh===!0)t.getVertexPosition(r,_n);else _n.fromBufferAttribute(s,r);_n.applyMatrix4(t.matrixWorld),this.expandByPoint(_n)}else{if(t.boundingBox!==void 0){if(t.boundingBox===null)t.computeBoundingBox();Cs.copy(t.boundingBox)}else{if(n.boundingBox===null)n.computeBoundingBox();Cs.copy(n.boundingBox)}Cs.applyMatrix4(t.matrixWorld),this.union(Cs)}}let i=t.children;for(let s=0,r=i.length;s<r;s++)this.expandByObject(i[s],e);return this}containsPoint(t){return t.x>=this.min.x&&t.x<=this.max.x&&t.y>=this.min.y&&t.y<=this.max.y&&t.z>=this.min.z&&t.z<=this.max.z}containsBox(t){return this.min.x<=t.min.x&&t.max.x<=this.max.x&&this.min.y<=t.min.y&&t.max.y<=this.max.y&&this.min.z<=t.min.z&&t.max.z<=this.max.z}getParameter(t,e){return e.set((t.x-this.min.x)/(this.max.x-this.min.x),(t.y-this.min.y)/(this.max.y-this.min.y),(t.z-this.min.z)/(this.max.z-this.min.z))}intersectsBox(t){return t.max.x>=this.min.x&&t.min.x<=this.max.x&&t.max.y>=this.min.y&&t.min.y<=this.max.y&&t.max.z>=this.min.z&&t.min.z<=this.max.z}intersectsSphere(t){return this.clampPoint(t.center,_n),_n.distanceToSquared(t.center)<=t.radius*t.radius}intersectsPlane(t){let e,n;if(t.normal.x>0)e=t.normal.x*this.min.x,n=t.normal.x*this.max.x;else e=t.normal.x*this.max.x,n=t.normal.x*this.min.x;if(t.normal.y>0)e+=t.normal.y*this.min.y,n+=t.normal.y*this.max.y;else e+=t.normal.y*this.max.y,n+=t.normal.y*this.min.y;if(t.normal.z>0)e+=t.normal.z*this.min.z,n+=t.normal.z*this.max.z;else e+=t.normal.z*this.max.z,n+=t.normal.z*this.min.z;return e<=-t.constant&&n>=-t.constant}intersectsTriangle(t){if(this.isEmpty())return!1;this.getCenter(Ki),Is.subVectors(this.max,Ki),Ti.subVectors(t.a,Ki),Ei.subVectors(t.b,Ki),Ri.subVectors(t.c,Ki),Hn.subVectors(Ei,Ti),Wn.subVectors(Ri,Ei),ni.subVectors(Ti,Ri);let e=[0,-Hn.z,Hn.y,0,-Wn.z,Wn.y,0,-ni.z,ni.y,Hn.z,0,-Hn.x,Wn.z,0,-Wn.x,ni.z,0,-ni.x,-Hn.y,Hn.x,0,-Wn.y,Wn.x,0,-ni.y,ni.x,0];if(!Kr(e,Ti,Ei,Ri,Is))return!1;if(e=[1,0,0,0,1,0,0,0,1],!Kr(e,Ti,Ei,Ri,Is))return!1;return Ps.crossVectors(Hn,Wn),e=[Ps.x,Ps.y,Ps.z],Kr(e,Ti,Ei,Ri,Is)}clampPoint(t,e){return e.copy(t).clamp(this.min,this.max)}distanceToPoint(t){return this.clampPoint(t,_n).distanceTo(t)}getBoundingSphere(t){if(this.isEmpty())t.makeEmpty();else this.getCenter(t.center),t.radius=this.getSize(_n).length()*0.5;return t}intersect(t){if(this.min.max(t.min),this.max.min(t.max),this.isEmpty())this.makeEmpty();return this}union(t){return this.min.min(t.min),this.max.max(t.max),this}applyMatrix4(t){if(this.isEmpty())return this;return Ln[0].set(this.min.x,this.min.y,this.min.z).applyMatrix4(t),Ln[1].set(this.min.x,this.min.y,this.max.z).applyMatrix4(t),Ln[2].set(this.min.x,this.max.y,this.min.z).applyMatrix4(t),Ln[3].set(this.min.x,this.max.y,this.max.z).applyMatrix4(t),Ln[4].set(this.max.x,this.min.y,this.min.z).applyMatrix4(t),Ln[5].set(this.max.x,this.min.y,this.max.z).applyMatrix4(t),Ln[6].set(this.max.x,this.max.y,this.min.z).applyMatrix4(t),Ln[7].set(this.max.x,this.max.y,this.max.z).applyMatrix4(t),this.setFromPoints(Ln),this}translate(t){return this.min.add(t),this.max.add(t),this}equals(t){return t.min.equals(this.min)&&t.max.equals(this.max)}toJSON(){return{min:this.min.toArray(),max:this.max.toArray()}}fromJSON(t){return this.min.fromArray(t.min),this.max.fromArray(t.max),this}}var Ln=[new W,new W,new W,new W,new W,new W,new W,new W],_n=new W,Cs=new ui,Ti=new W,Ei=new W,Ri=new W,Hn=new W,Wn=new W,ni=new W,Ki=new W,Is=new W,Ps=new W,ii=new W;function Kr(t,e,n,i,s){for(let r=0,a=t.length-3;r<=a;r+=3){ii.fromArray(t,r);let o=s.x*Math.abs(ii.x)+s.y*Math.abs(ii.y)+s.z*Math.abs(ii.z),l=e.dot(ii),c=n.dot(ii),p=i.dot(ii);if(Math.max(-Math.max(l,c,p),Math.min(l,c,p))>o)return!1}return!0}var Pe=new W,Ls=new fe,kh=0;class Ke extends On{constructor(t,e,n=!1){super();if(Array.isArray(t))throw TypeError("THREE.BufferAttribute: array should be a Typed Array.");this.isBufferAttribute=!0,Object.defineProperty(this,"id",{value:kh++}),this.name="",this.array=t,this.itemSize=e,this.count=t!==void 0?t.length/e:0,this.normalized=n,this.usage=35044,this.updateRanges=[],this.gpuType=1015,this.version=0,this._normalizationScale=t!==void 0?oo(t):void 0}onUploadCallback(){}set needsUpdate(t){if(t===!0)this.version++}setUsage(t){return this.usage=t,this}addUpdateRange(t,e){this.updateRanges.push({start:t,count:e})}clearUpdateRanges(){this.updateRanges.length=0}copy(t){return this.name=t.name,this.array=new t.array.constructor(t.array),this.itemSize=t.itemSize,this.count=t.count,this.normalized=t.normalized,this.usage=t.usage,this.gpuType=t.gpuType,this._normalizationScale=t._normalizationScale,this}copyAt(t,e,n){t*=this.itemSize,n*=e.itemSize;for(let i=0,s=this.itemSize;i<s;i++)this.array[t+i]=e.array[n+i];return this}copyArray(t){return this.array.set(t),this}applyMatrix3(t){if(this.itemSize===2)for(let e=0,n=this.count;e<n;e++)Ls.fromBufferAttribute(this,e),Ls.applyMatrix3(t),this.setXY(e,Ls.x,Ls.y);else if(this.itemSize===3)for(let e=0,n=this.count;e<n;e++)Pe.fromBufferAttribute(this,e),Pe.applyMatrix3(t),this.setXYZ(e,Pe.x,Pe.y,Pe.z);return this}applyMatrix4(t){for(let e=0,n=this.count;e<n;e++)Pe.fromBufferAttribute(this,e),Pe.applyMatrix4(t),this.setXYZ(e,Pe.x,Pe.y,Pe.z);return this}applyNormalMatrix(t){for(let e=0,n=this.count;e<n;e++)Pe.fromBufferAttribute(this,e),Pe.applyNormalMatrix(t),this.setXYZ(e,Pe.x,Pe.y,Pe.z);return this}transformDirection(t){for(let e=0,n=this.count;e<n;e++)Pe.fromBufferAttribute(this,e),Pe.transformDirection(t),this.setXYZ(e,Pe.x,Pe.y,Pe.z);return this}set(t,e=0){return this.array.set(t,e),this}getComponent(t,e){let n=this.array[t*this.itemSize+e];if(this.normalized)n=this._denormalize(n);return n}setComponent(t,e,n){if(this.normalized)n=this._normalize(n);return this.array[t*this.itemSize+e]=n,this}getX(t){let e=this.array[t*this.itemSize];if(this.normalized)e=this._denormalize(e);return e}setX(t,e){if(this.normalized)e=this._normalize(e);return this.array[t*this.itemSize]=e,this}getY(t){let e=this.array[t*this.itemSize+1];if(this.normalized)e=this._denormalize(e);return e}setY(t,e){if(this.normalized)e=this._normalize(e);return this.array[t*this.itemSize+1]=e,this}getZ(t){let e=this.array[t*this.itemSize+2];if(this.normalized)e=this._denormalize(e);return e}setZ(t,e){if(this.normalized)e=this._normalize(e);return this.array[t*this.itemSize+2]=e,this}getW(t){let e=this.array[t*this.itemSize+3];if(this.normalized)e=this._denormalize(e);return e}setW(t,e){if(this.normalized)e=this._normalize(e);return this.array[t*this.itemSize+3]=e,this}setXY(t,e,n){if(t*=this.itemSize,this.normalized)e=this._normalize(e),n=this._normalize(n);return this.array[t+0]=e,this.array[t+1]=n,this}setXYZ(t,e,n,i){if(t*=this.itemSize,this.normalized)e=this._normalize(e),n=this._normalize(n),i=this._normalize(i);return this.array[t+0]=e,this.array[t+1]=n,this.array[t+2]=i,this}setXYZW(t,e,n,i,s){if(t*=this.itemSize,this.normalized)e=this._normalize(e),n=this._normalize(n),i=this._normalize(i),s=this._normalize(s);return this.array[t+0]=e,this.array[t+1]=n,this.array[t+2]=i,this.array[t+3]=s,this}onUpload(t){return this.onUploadCallback=t,this}clone(){return new this.constructor(this.array,this.itemSize).copy(this)}toJSON(){let t={itemSize:this.itemSize,type:this.array.constructor.name,array:Array.from(this.array),normalized:this.normalized};return t.name=this.name,t.usage=this.usage,t.gpuType=this.gpuType,t}dispose(){this.dispatchEvent({type:"dispose"})}_denormalize(t){let e=this._normalizationScale;return e!==void 0?Math.max(t/e,-1):Rh(t,this.array)}_normalize(t){let e=this._normalizationScale;return e!==void 0?Math.round(t*e):Ch(t,this.array)}}class dr extends Ke{constructor(t,e,n){super(new Uint16Array(t),e,n)}}class fr extends Ke{constructor(t,e,n){super(new Uint32Array(t),e,n)}}class sn extends Ke{constructor(t,e,n){super(new Float32Array(t),e,n)}}var Gh=new ui,Qi=new W,Qr=new W;class di{constructor(t=new W,e=-1){this.isSphere=!0,this.center=t,this.radius=e}set(t,e){return this.center.copy(t),this.radius=e,this}setFromPoints(t,e){let n=this.center;if(e!==void 0)n.copy(e);else Gh.setFromPoints(t).getCenter(n);let i=0;for(let s=0,r=t.length;s<r;s++)i=Math.max(i,n.distanceToSquared(t[s]));return this.radius=Math.sqrt(i),this}copy(t){return this.center.copy(t.center),this.radius=t.radius,this}isEmpty(){return this.radius<0}makeEmpty(){return this.center.set(0,0,0),this.radius=-1,this}containsPoint(t){return t.distanceToSquared(this.center)<=this.radius*this.radius}distanceToPoint(t){return t.distanceTo(this.center)-this.radius}intersectsSphere(t){let e=this.radius+t.radius;return t.center.distanceToSquared(this.center)<=e*e}intersectsBox(t){return t.intersectsSphere(this)}intersectsPlane(t){return Math.abs(t.distanceToPoint(this.center))<=this.radius}clampPoint(t,e){let n=this.center.distanceToSquared(t);if(e.copy(t),n>this.radius*this.radius)e.sub(this.center).normalize(),e.multiplyScalar(this.radius).add(this.center);return e}getBoundingBox(t){if(this.isEmpty())return t.makeEmpty(),t;return t.set(this.center,this.center),t.expandByScalar(this.radius),t}applyMatrix4(t){return this.center.applyMatrix4(t),this.radius=this.radius*t.getMaxScaleOnAxis(),this}translate(t){return this.center.add(t),this}expandByPoint(t){if(this.isEmpty())return this.center.copy(t),this.radius=0,this;Qi.subVectors(t,this.center);let e=Qi.lengthSq();if(e>this.radius*this.radius){let n=Math.sqrt(e),i=(n-this.radius)*0.5;this.center.addScaledVector(Qi,i/n),this.radius+=i}return this}union(t){if(t.isEmpty())return this;if(this.isEmpty())return this.copy(t),this;if(this.center.equals(t.center)===!0)this.radius=Math.max(this.radius,t.radius);else Qr.subVectors(t.center,this.center).setLength(t.radius),this.expandByPoint(Qi.copy(t.center).add(Qr)),this.expandByPoint(Qi.copy(t.center).sub(Qr));return this}equals(t){return t.center.equals(this.center)&&t.radius===this.radius}clone(){return new this.constructor().copy(this)}toJSON(){return{radius:this.radius,center:this.center.toArray()}}fromJSON(t){return this.radius=t.radius,this.center.fromArray(t.center),this}}var Hh=0,an=new Se,jr=new Xe,Ci=new W,nn=new ui,ji=new ui,Be=new W;class Ee extends On{constructor(){super();this.isBufferGeometry=!0,Object.defineProperty(this,"id",{value:Hh++}),this.uuid=ls(),this.name="",this.type="BufferGeometry",this.index=null,this.indirect=null,this.indirectOffset=0,this.attributes={},this.morphAttributes={},this.morphTargetsRelative=!1,this.groups=[],this.boundingBox=null,this.boundingSphere=null,this.drawRange={start:0,count:1/0},this.userData={},this._transformed=!1}getIndex(){return this.index}setIndex(t){if(Array.isArray(t))this.index=new((Ah(t))?fr:dr)(t,1);else this.index=t;return this}setIndirect(t,e=0){return this.indirect=t,this.indirectOffset=e,this}getIndirect(){return this.indirect}getAttribute(t){return this.attributes[t]}setAttribute(t,e){return this.attributes[t]=e,this}deleteAttribute(t){return delete this.attributes[t],this}hasAttribute(t){return this.attributes[t]!==void 0}addGroup(t,e,n=0){this.groups.push({start:t,count:e,materialIndex:n})}clearGroups(){this.groups=[]}setDrawRange(t,e){this.drawRange.start=t,this.drawRange.count=e}applyMatrix4(t){let e=this.attributes.position;if(e!==void 0)e.applyMatrix4(t),e.needsUpdate=!0;let n=this.attributes.normal;if(n!==void 0){let s=new Zt().getNormalMatrix(t);n.applyNormalMatrix(s),n.needsUpdate=!0}let i=this.attributes.tangent;if(i!==void 0)i.transformDirection(t),i.needsUpdate=!0;if(this.boundingBox!==null)this.computeBoundingBox();if(this.boundingSphere!==null)this.computeBoundingSphere();return this._transformed=!0,this}applyQuaternion(t){return an.makeRotationFromQuaternion(t),this.applyMatrix4(an),this}rotateX(t){return an.makeRotationX(t),this.applyMatrix4(an),this}rotateY(t){return an.makeRotationY(t),this.applyMatrix4(an),this}rotateZ(t){return an.makeRotationZ(t),this.applyMatrix4(an),this}translate(t,e,n){return an.makeTranslation(t,e,n),this.applyMatrix4(an),this}scale(t,e,n){return an.makeScale(t,e,n),this.applyMatrix4(an),this}lookAt(t){return jr.lookAt(t),jr.updateMatrix(),this.applyMatrix4(jr.matrix),this}center(){return this.computeBoundingBox(),this.boundingBox.getCenter(Ci).negate(),this.translate(Ci.x,Ci.y,Ci.z),this}setFromPoints(t){let e=this.getAttribute("position");if(e===void 0){let n=[];for(let i=0,s=t.length;i<s;i++){let r=t[i];n.push(r.x,r.y,r.z||0)}this.setAttribute("position",new sn(n,3))}else{let n=Math.min(t.length,e.count);for(let i=0;i<n;i++){let s=t[i];e.setXYZ(i,s.x,s.y,s.z||0)}if(t.length>e.count)Wt("BufferGeometry: Buffer size too small for points data. Use .dispose() and create a new geometry.");e.needsUpdate=!0}return this}computeBoundingBox(){if(this.boundingBox===null)this.boundingBox=new ui;let t=this.attributes.position,e=this.morphAttributes.position;if(t&&t.isGLBufferAttribute){Yt("BufferGeometry.computeBoundingBox(): GLBufferAttribute requires a manual bounding box.",this),this.boundingBox.set(new W(-1/0,-1/0,-1/0),new W(1/0,1/0,1/0));return}if(t!==void 0){if(this.boundingBox.setFromBufferAttribute(t),e)for(let n=0,i=e.length;n<i;n++){let s=e[n];if(nn.setFromBufferAttribute(s),this.morphTargetsRelative)Be.addVectors(this.boundingBox.min,nn.min),this.boundingBox.expandByPoint(Be),Be.addVectors(this.boundingBox.max,nn.max),this.boundingBox.expandByPoint(Be);else this.boundingBox.expandByPoint(nn.min),this.boundingBox.expandByPoint(nn.max)}}else this.boundingBox.makeEmpty();if(isNaN(this.boundingBox.min.x)||isNaN(this.boundingBox.min.y)||isNaN(this.boundingBox.min.z))Yt('BufferGeometry.computeBoundingBox(): Computed min/max have NaN values. The "position" attribute is likely to have NaN values.',this)}computeBoundingSphere(){if(this.boundingSphere===null)this.boundingSphere=new di;let t=this.attributes.position,e=this.morphAttributes.position;if(t&&t.isGLBufferAttribute){Yt("BufferGeometry.computeBoundingSphere(): GLBufferAttribute requires a manual bounding sphere.",this),this.boundingSphere.set(new W,1/0);return}if(t){let n=this.boundingSphere.center;if(nn.setFromBufferAttribute(t),e)for(let s=0,r=e.length;s<r;s++){let a=e[s];if(ji.setFromBufferAttribute(a),this.morphTargetsRelative)Be.addVectors(nn.min,ji.min),nn.expandByPoint(Be),Be.addVectors(nn.max,ji.max),nn.expandByPoint(Be);else nn.expandByPoint(ji.min),nn.expandByPoint(ji.max)}nn.getCenter(n);let i=0;for(let s=0,r=t.count;s<r;s++)Be.fromBufferAttribute(t,s),i=Math.max(i,n.distanceToSquared(Be));if(e)for(let s=0,r=e.length;s<r;s++){let a=e[s],o=this.morphTargetsRelative;for(let l=0,c=a.count;l<c;l++){if(Be.fromBufferAttribute(a,l),o)Ci.fromBufferAttribute(t,l),Be.add(Ci);i=Math.max(i,n.distanceToSquared(Be))}}if(this.boundingSphere.radius=Math.sqrt(i),isNaN(this.boundingSphere.radius))Yt('BufferGeometry.computeBoundingSphere(): Computed radius is NaN. The "position" attribute is likely to have NaN values.',this)}}computeTangents(){let t=this.index,e=this.attributes;if(t===null||e.position===void 0||e.normal===void 0||e.uv===void 0){Yt("BufferGeometry: .computeTangents() failed. Missing required attributes (index, position, normal or uv)");return}let{position:n,normal:i,uv:s}=e,r=this.getAttribute("tangent");if(r===void 0||r.count!==n.count)r=new Ke(new Float32Array(4*n.count),4),this.setAttribute("tangent",r);let a=[],o=[],l=new Uint8Array(n.count);for(let y=0;y<n.count;y++)a[y]=new W,o[y]=new W;let c=new W,p=new W,h=new W,u=new fe,d=new fe,v=new fe,w=new W,f=new W;function m(y,D,q){l[y]=l[D]=l[q]=1,c.fromBufferAttribute(n,y),p.fromBufferAttribute(n,D),h.fromBufferAttribute(n,q),u.fromBufferAttribute(s,y),d.fromBufferAttribute(s,D),v.fromBufferAttribute(s,q),p.sub(c),h.sub(c),d.sub(u),v.sub(u);let N=1/(d.x*v.y-v.x*d.y);if(!isFinite(N))return;w.copy(p).multiplyScalar(v.y).addScaledVector(h,-d.y).multiplyScalar(N),f.copy(h).multiplyScalar(d.x).addScaledVector(p,-v.x).multiplyScalar(N),a[y].add(w),a[D].add(w),a[q].add(w),o[y].add(f),o[D].add(f),o[q].add(f)}let M=this.groups;if(M.length===0)M=[{start:0,count:t.count}];for(let y=0,D=M.length;y<D;++y){let q=M[y],{start:N,count:X}=q;for(let rt=N,R=N+X;rt<R;rt+=3)m(t.getX(rt+0),t.getX(rt+1),t.getX(rt+2))}let T=new W,S=new W,E=new W,P=new W;function g(y){E.fromBufferAttribute(i,y),P.copy(E);let D=a[y];T.copy(D),T.sub(E.multiplyScalar(E.dot(D))).normalize(),S.crossVectors(P,D);let N=S.dot(o[y])<0?-1:1;r.setXYZW(y,T.x,T.y,T.z,N)}for(let y=0,D=n.count;y<D;++y)if(l[y])g(y);this._transformed=!0}computeVertexNormals(){let t=this.index,e=this.getAttribute("position");if(e!==void 0){let n=this.getAttribute("normal");if(n===void 0||n.count!==e.count)n=new Ke(new Float32Array(e.count*3),3),this.setAttribute("normal",n);else for(let h=0,u=n.count;h<u;h++)n.setXYZ(h,0,0,0);let i=new W,s=new W,r=new W,a=new W,o=new W,l=new W,c=new W,p=new W;if(t)for(let h=0,u=t.count;h<u;h+=3){let d=t.getX(h+0),v=t.getX(h+1),w=t.getX(h+2);i.fromBufferAttribute(e,d),s.fromBufferAttribute(e,v),r.fromBufferAttribute(e,w),c.subVectors(r,s),p.subVectors(i,s),c.cross(p),a.fromBufferAttribute(n,d),o.fromBufferAttribute(n,v),l.fromBufferAttribute(n,w),a.add(c),o.add(c),l.add(c),n.setXYZ(d,a.x,a.y,a.z),n.setXYZ(v,o.x,o.y,o.z),n.setXYZ(w,l.x,l.y,l.z)}else for(let h=0,u=e.count;h<u;h+=3)i.fromBufferAttribute(e,h+0),s.fromBufferAttribute(e,h+1),r.fromBufferAttribute(e,h+2),c.subVectors(r,s),p.subVectors(i,s),c.cross(p),n.setXYZ(h+0,c.x,c.y,c.z),n.setXYZ(h+1,c.x,c.y,c.z),n.setXYZ(h+2,c.x,c.y,c.z);this.normalizeNormals(),n.needsUpdate=!0}}normalizeNormals(){let t=this.attributes.normal;for(let e=0,n=t.count;e<n;e++)Be.fromBufferAttribute(t,e),Be.normalize(),t.setXYZ(e,Be.x,Be.y,Be.z)}toNonIndexed(){function t(a,o){let{array:l,itemSize:c,normalized:p}=a,h=new l.constructor(o.length*c),u=0,d=0;for(let v=0,w=o.length;v<w;v++){if(a.isInterleavedBufferAttribute)u=o[v]*a.data.stride+a.offset;else u=o[v]*c;for(let f=0;f<c;f++)h[d++]=l[u++]}return new Ke(h,c,p)}if(this.index===null)return Wt("BufferGeometry.toNonIndexed(): BufferGeometry is already non-indexed."),this;let e=new Ee,n=this.index.array,i=this.attributes;for(let a in i){let o=i[a],l=t(o,n);e.setAttribute(a,l)}let s=this.morphAttributes;for(let a in s){let o=[],l=s[a];for(let c=0,p=l.length;c<p;c++){let h=l[c],u=t(h,n);o.push(u)}e.morphAttributes[a]=o}e.morphTargetsRelative=this.morphTargetsRelative;let r=this.groups;for(let a=0,o=r.length;a<o;a++){let l=r[a];e.addGroup(l.start,l.count,l.materialIndex)}return e}toJSON(){let t={metadata:{version:4.7,type:"BufferGeometry",generator:"BufferGeometry.toJSON"}};if(t.uuid=this.uuid,t.type=this.parameters!==void 0&&this._transformed===!0?"BufferGeometry":this.type,t.name=this.name,Object.keys(this.userData).length>0)t.userData=this.userData;if(this.parameters!==void 0&&this._transformed!==!0){let o=this.parameters;for(let l in o)if(o[l]!==void 0)t[l]=o[l];return t}t.data={attributes:{}};let e=this.index;if(e!==null)t.data.index={type:e.array.constructor.name,array:Array.prototype.slice.call(e.array)};let n=this.attributes;for(let o in n){let l=n[o];t.data.attributes[o]=l.toJSON(t.data)}let i={},s=!1;for(let o in this.morphAttributes){let l=this.morphAttributes[o],c=[];for(let p=0,h=l.length;p<h;p++){let u=l[p];c.push(u.toJSON(t.data))}if(c.length>0)i[o]=c,s=!0}if(s)t.data.morphAttributes=i,t.data.morphTargetsRelative=this.morphTargetsRelative;let r=this.groups;if(r.length>0)t.data.groups=JSON.parse(JSON.stringify(r));let a=this.boundingSphere;if(a!==null)t.data.boundingSphere=a.toJSON();return t}clone(){return new this.constructor().copy(this)}copy(t){this.index=null,this.attributes={},this.morphAttributes={},this.groups=[],this.boundingBox=null,this.boundingSphere=null;let e={};this.name=t.name;let n=t.index;if(n!==null)this.setIndex(n.clone());let i=t.attributes;for(let l in i){let c=i[l];this.setAttribute(l,c.clone(e))}let s=t.morphAttributes;for(let l in s){let c=[],p=s[l];for(let h=0,u=p.length;h<u;h++)c.push(p[h].clone(e));this.morphAttributes[l]=c}this.morphTargetsRelative=t.morphTargetsRelative;let r=t.groups;for(let l=0,c=r.length;l<c;l++){let p=r[l];this.addGroup(p.start,p.count,p.materialIndex)}let a=t.boundingBox;if(a!==null)this.boundingBox=a.clone();let o=t.boundingSphere;if(o!==null)this.boundingSphere=o.clone();return this.drawRange.start=t.drawRange.start,this.drawRange.count=t.drawRange.count,this.userData=t.userData,this._transformed=t._transformed,this}dispose(){this.dispatchEvent({type:"dispose"})}}var ta=new W,Wh=new W,Vh=new Zt;class Mn{constructor(t=new W(1,0,0),e=0){this.isPlane=!0,this.normal=t,this.constant=e}set(t,e){return this.normal.copy(t),this.constant=e,this}setComponents(t,e,n,i){return this.normal.set(t,e,n),this.constant=i,this}setFromNormalAndCoplanarPoint(t,e){return this.normal.copy(t),this.constant=-e.dot(this.normal),this}setFromCoplanarPoints(t,e,n){let i=ta.subVectors(n,e).cross(Wh.subVectors(t,e)).normalize();return this.setFromNormalAndCoplanarPoint(i,t),this}copy(t){return this.normal.copy(t.normal),this.constant=t.constant,this}normalize(){let t=1/this.normal.length();return this.normal.multiplyScalar(t),this.constant*=t,this}negate(){return this.constant*=-1,this.normal.negate(),this}distanceToPoint(t){return this.normal.dot(t)+this.constant}distanceToSphere(t){return this.distanceToPoint(t.center)-t.radius}projectPoint(t,e){return e.copy(t).addScaledVector(this.normal,-this.distanceToPoint(t))}intersectLine(t,e,n=!0){let i=t.delta(ta),s=this.normal.dot(i);if(s===0){if(this.distanceToPoint(t.start)===0)return e.copy(t.start);return null}let r=-(t.start.dot(this.normal)+this.constant)/s;if(n===!0&&(r<0||r>1))return null;return e.copy(t.start).addScaledVector(i,r)}intersectsLine(t){let e=this.distanceToPoint(t.start),n=this.distanceToPoint(t.end);return e<0&&n>0||n<0&&e>0}intersectsBox(t){return t.intersectsPlane(this)}intersectsSphere(t){return t.intersectsPlane(this)}coplanarPoint(t){return t.copy(this.normal).multiplyScalar(-this.constant)}applyMatrix4(t,e){let n=e||Vh.getNormalMatrix(t),i=this.coplanarPoint(ta).applyMatrix4(t),s=this.normal.applyMatrix3(n).normalize();return this.constant=-i.dot(s),this}translate(t){return this.constant-=t.dot(this.normal),this}equals(t){return t.normal.equals(this.normal)&&t.constant===this.constant}clone(){return new this.constructor().copy(this)}toJSON(){return{normal:this.normal.toArray(),constant:this.constant}}fromJSON(t){return this.normal.fromArray(t.normal),this.constant=t.constant,this}}var Xh=0;class $n extends On{constructor(){super();this.isMaterial=!0,Object.defineProperty(this,"id",{value:Xh++}),this.uuid=ls(),this.name="",this.type="Material",this.blending=1,this.side=0,this.vertexColors=!1,this.opacity=1,this.transparent=!1,this.alphaHash=!1,this.blendSrc=204,this.blendDst=205,this.blendEquation=100,this.blendSrcAlpha=null,this.blendDstAlpha=null,this.blendEquationAlpha=null,this.blendColor=new le(0,0,0),this.blendAlpha=0,this.depthFunc=3,this.depthTest=!0,this.depthWrite=!0,this.stencilWriteMask=255,this.stencilFunc=519,this.stencilRef=0,this.stencilFuncMask=255,this.stencilFail=7680,this.stencilZFail=7680,this.stencilZPass=7680,this.stencilWrite=!1,this.clippingPlanes=null,this.clipIntersection=!1,this.clipShadows=!1,this.shadowSide=null,this.colorWrite=!0,this.precision=null,this.polygonOffset=!1,this.polygonOffsetFactor=0,this.polygonOffsetUnits=0,this.dithering=!1,this.alphaToCoverage=!1,this.premultipliedAlpha=!1,this.forceSinglePass=!1,this.allowOverride=!0,this.visible=!0,this.toneMapped=!0,this.userData={},this.version=0,this._alphaTest=0}get alphaTest(){return this._alphaTest}set alphaTest(t){if(this._alphaTest>0!==t>0)this.version++;this._alphaTest=t}onBeforeRender(){}onBeforeCompile(){}customProgramCacheKey(){return this.onBeforeCompile.toString()}setValues(t){if(t===void 0)return;for(let e in t){let n=t[e];if(n===void 0){Wt(`Material: parameter '${e}' has value of undefined.`);continue}let i=this[e];if(i===void 0){Wt(`Material: '${e}' is not a property of THREE.${this.type}.`);continue}if(i&&i.isColor)i.set(n);else if(i&&i.isVector2&&(n&&n.isVector2)||i&&i.isEuler&&(n&&n.isEuler)||i&&i.isVector3&&(n&&n.isVector3))i.copy(n);else this[e]=n}}toJSON(t){let e=t===void 0||typeof t==="string";if(e)t={textures:{},images:{}};let n={metadata:{version:4.7,type:"Material",generator:"Material.toJSON"}};if(n.uuid=this.uuid,n.type=this.type,n.blending=this.blending,n.side=this.side,n.shadowSide=this.shadowSide,n.vertexColors=this.vertexColors,n.opacity=this.opacity,n.transparent=this.transparent,n.blendSrc=this.blendSrc,n.blendDst=this.blendDst,n.blendEquation=this.blendEquation,n.blendSrcAlpha=this.blendSrcAlpha,n.blendDstAlpha=this.blendDstAlpha,n.blendEquationAlpha=this.blendEquationAlpha,n.blendColor=this.blendColor.getHex(),n.blendAlpha=this.blendAlpha,n.depthFunc=this.depthFunc,n.depthTest=this.depthTest,n.depthWrite=this.depthWrite,n.colorWrite=this.colorWrite,n.clipIntersection=this.clipIntersection,n.clipShadows=this.clipShadows,n.stencilWriteMask=this.stencilWriteMask,n.stencilFunc=this.stencilFunc,n.stencilRef=this.stencilRef,n.stencilFuncMask=this.stencilFuncMask,n.stencilFail=this.stencilFail,n.stencilZFail=this.stencilZFail,n.stencilZPass=this.stencilZPass,n.stencilWrite=this.stencilWrite,n.polygonOffset=this.polygonOffset,n.polygonOffsetFactor=this.polygonOffsetFactor,n.polygonOffsetUnits=this.polygonOffsetUnits,n.dithering=this.dithering,n.alphaTest=this.alphaTest,n.alphaHash=this.alphaHash,n.alphaToCoverage=this.alphaToCoverage,n.premultipliedAlpha=this.premultipliedAlpha,n.forceSinglePass=this.forceSinglePass,n.allowOverride=this.allowOverride,n.visible=this.visible,n.toneMapped=this.toneMapped,n.name=this.name,this.color&&this.color.isColor)n.color=this.color.getHex();if(this.roughness!==void 0)n.roughness=this.roughness;if(this.metalness!==void 0)n.metalness=this.metalness;if(this.sheen!==void 0)n.sheen=this.sheen;if(this.sheenColor&&this.sheenColor.isColor)n.sheenColor=this.sheenColor.getHex();if(this.sheenRoughness!==void 0)n.sheenRoughness=this.sheenRoughness;if(this.emissive&&this.emissive.isColor)n.emissive=this.emissive.getHex();if(this.emissiveIntensity!==void 0)n.emissiveIntensity=this.emissiveIntensity;if(this.specular&&this.specular.isColor)n.specular=this.specular.getHex();if(this.specularIntensity!==void 0)n.specularIntensity=this.specularIntensity;if(this.specularColor&&this.specularColor.isColor)n.specularColor=this.specularColor.getHex();if(this.shininess!==void 0)n.shininess=this.shininess;if(this.clearcoat!==void 0)n.clearcoat=this.clearcoat;if(this.clearcoatRoughness!==void 0)n.clearcoatRoughness=this.clearcoatRoughness;if(this.diffuseRoughness!==void 0)n.diffuseRoughness=this.diffuseRoughness;if(this.diffuseRoughnessMap&&this.diffuseRoughnessMap.isTexture)n.diffuseRoughnessMap=this.diffuseRoughnessMap.toJSON(t).uuid;if(this.clearcoatMap&&this.clearcoatMap.isTexture)n.clearcoatMap=this.clearcoatMap.toJSON(t).uuid;if(this.clearcoatRoughnessMap&&this.clearcoatRoughnessMap.isTexture)n.clearcoatRoughnessMap=this.clearcoatRoughnessMap.toJSON(t).uuid;if(this.clearcoatNormalMap&&this.clearcoatNormalMap.isTexture)n.clearcoatNormalMap=this.clearcoatNormalMap.toJSON(t).uuid,n.clearcoatNormalScale=this.clearcoatNormalScale.toArray();if(this.sheenColorMap&&this.sheenColorMap.isTexture)n.sheenColorMap=this.sheenColorMap.toJSON(t).uuid;if(this.sheenRoughnessMap&&this.sheenRoughnessMap.isTexture)n.sheenRoughnessMap=this.sheenRoughnessMap.toJSON(t).uuid;if(this.dispersion!==void 0)n.dispersion=this.dispersion;if(this.retroreflectivity!==void 0)n.retroreflectivity=this.retroreflectivity;if(this.iridescence!==void 0)n.iridescence=this.iridescence;if(this.iridescenceIOR!==void 0)n.iridescenceIOR=this.iridescenceIOR;if(this.iridescenceThicknessMinimum!==void 0)n.iridescenceThicknessMinimum=this.iridescenceThicknessMinimum;if(this.iridescenceThicknessMaximum!==void 0)n.iridescenceThicknessMaximum=this.iridescenceThicknessMaximum;if(this.iridescenceMap&&this.iridescenceMap.isTexture)n.iridescenceMap=this.iridescenceMap.toJSON(t).uuid;if(this.iridescenceThicknessMap&&this.iridescenceThicknessMap.isTexture)n.iridescenceThicknessMap=this.iridescenceThicknessMap.toJSON(t).uuid;if(this.anisotropy!==void 0)n.anisotropy=this.anisotropy;if(this.anisotropyRotation!==void 0)n.anisotropyRotation=this.anisotropyRotation;if(this.anisotropyMap&&this.anisotropyMap.isTexture)n.anisotropyMap=this.anisotropyMap.toJSON(t).uuid;if(this.map&&this.map.isTexture)n.map=this.map.toJSON(t).uuid;if(this.matcap&&this.matcap.isTexture)n.matcap=this.matcap.toJSON(t).uuid;if(this.alphaMap&&this.alphaMap.isTexture)n.alphaMap=this.alphaMap.toJSON(t).uuid;if(this.lightMap&&this.lightMap.isTexture)n.lightMap=this.lightMap.toJSON(t).uuid,n.lightMapIntensity=this.lightMapIntensity;if(this.aoMap&&this.aoMap.isTexture)n.aoMap=this.aoMap.toJSON(t).uuid,n.aoMapIntensity=this.aoMapIntensity;if(this.bumpMap&&this.bumpMap.isTexture)n.bumpMap=this.bumpMap.toJSON(t).uuid,n.bumpScale=this.bumpScale;if(this.normalMap&&this.normalMap.isTexture)n.normalMap=this.normalMap.toJSON(t).uuid,n.normalMapType=this.normalMapType,n.normalScale=this.normalScale.toArray();if(this.displacementMap&&this.displacementMap.isTexture)n.displacementMap=this.displacementMap.toJSON(t).uuid,n.displacementScale=this.displacementScale,n.displacementBias=this.displacementBias;if(this.roughnessMap&&this.roughnessMap.isTexture)n.roughnessMap=this.roughnessMap.toJSON(t).uuid;if(this.metalnessMap&&this.metalnessMap.isTexture)n.metalnessMap=this.metalnessMap.toJSON(t).uuid;if(this.emissiveMap&&this.emissiveMap.isTexture)n.emissiveMap=this.emissiveMap.toJSON(t).uuid;if(this.specularMap&&this.specularMap.isTexture)n.specularMap=this.specularMap.toJSON(t).uuid;if(this.specularIntensityMap&&this.specularIntensityMap.isTexture)n.specularIntensityMap=this.specularIntensityMap.toJSON(t).uuid;if(this.specularColorMap&&this.specularColorMap.isTexture)n.specularColorMap=this.specularColorMap.toJSON(t).uuid;if(this.envMap&&this.envMap.isTexture){if(n.envMap=this.envMap.toJSON(t).uuid,this.combine!==void 0)n.combine=this.combine}if(this.envMapRotation!==void 0)n.envMapRotation=this.envMapRotation.toArray();if(this.envMapIntensity!==void 0)n.envMapIntensity=this.envMapIntensity;if(this.reflectivity!==void 0)n.reflectivity=this.reflectivity;if(this.refractionRatio!==void 0)n.refractionRatio=this.refractionRatio;if(this.gradientMap&&this.gradientMap.isTexture)n.gradientMap=this.gradientMap.toJSON(t).uuid;if(this.transmission!==void 0)n.transmission=this.transmission;if(this.transmissionMap&&this.transmissionMap.isTexture)n.transmissionMap=this.transmissionMap.toJSON(t).uuid;if(this.thickness!==void 0)n.thickness=this.thickness;if(this.thicknessMap&&this.thicknessMap.isTexture)n.thicknessMap=this.thicknessMap.toJSON(t).uuid;if(this.attenuationDistance!==void 0)n.attenuationDistance=this.attenuationDistance;if(this.attenuationColor!==void 0)n.attenuationColor=this.attenuationColor.getHex();if(this.size!==void 0)n.size=this.size;if(this.sizeAttenuation!==void 0)n.sizeAttenuation=this.sizeAttenuation;if(Array.isArray(this.clippingPlanes)&&this.clippingPlanes.length>0)n.clippingPlanes=this.clippingPlanes.map((s)=>s.toJSON());if(this.rotation!==void 0)n.rotation=this.rotation;if(this.depthPacking!==void 0)n.depthPacking=this.depthPacking;if(this.linewidth!==void 0)n.linewidth=this.linewidth;if(this.linecap!==void 0)n.linecap=this.linecap;if(this.linejoin!==void 0)n.linejoin=this.linejoin;if(this.dashSize!==void 0)n.dashSize=this.dashSize;if(this.gapSize!==void 0)n.gapSize=this.gapSize;if(this.scale!==void 0)n.scale=this.scale;if(this.wireframe!==void 0)n.wireframe=this.wireframe;if(this.wireframeLinewidth!==void 0)n.wireframeLinewidth=this.wireframeLinewidth;if(this.wireframeLinecap!==void 0)n.wireframeLinecap=this.wireframeLinecap;if(this.wireframeLinejoin!==void 0)n.wireframeLinejoin=this.wireframeLinejoin;if(this.flatShading!==void 0)n.flatShading=this.flatShading;if(this.fog!==void 0)n.fog=this.fog;if(Object.keys(this.userData).length>0)n.userData=this.userData;function i(s){let r=[];for(let a in s){let o=s[a];delete o.metadata,r.push(o)}return r}if(e){let s=i(t.textures),r=i(t.images);if(s.length>0)n.textures=s;if(r.length>0)n.images=r}return n}fromJSON(t,e){if(t.uuid!==void 0)this.uuid=t.uuid;if(t.name!==void 0)this.name=t.name;if(t.color!==void 0&&this.color!==void 0)this.color.setHex(t.color);if(t.roughness!==void 0)this.roughness=t.roughness;if(t.metalness!==void 0)this.metalness=t.metalness;if(t.sheen!==void 0)this.sheen=t.sheen;if(t.sheenColor!==void 0)this.sheenColor=new le().setHex(t.sheenColor);if(t.sheenRoughness!==void 0)this.sheenRoughness=t.sheenRoughness;if(t.emissive!==void 0&&this.emissive!==void 0)this.emissive.setHex(t.emissive);if(t.specular!==void 0&&this.specular!==void 0)this.specular.setHex(t.specular);if(t.specularIntensity!==void 0)this.specularIntensity=t.specularIntensity;if(t.specularColor!==void 0&&this.specularColor!==void 0)this.specularColor.setHex(t.specularColor);if(t.shininess!==void 0)this.shininess=t.shininess;if(t.clearcoat!==void 0)this.clearcoat=t.clearcoat;if(t.clearcoatRoughness!==void 0)this.clearcoatRoughness=t.clearcoatRoughness;if(t.diffuseRoughness!==void 0)this.diffuseRoughness=t.diffuseRoughness;if(t.dispersion!==void 0)this.dispersion=t.dispersion;if(t.retroreflectivity!==void 0)this.retroreflectivity=t.retroreflectivity;if(t.iridescence!==void 0)this.iridescence=t.iridescence;if(t.iridescenceIOR!==void 0)this.iridescenceIOR=t.iridescenceIOR;if(t.iridescenceThicknessMinimum!==void 0)this.iridescenceThicknessMinimum=t.iridescenceThicknessMinimum;if(t.iridescenceThicknessMaximum!==void 0)this.iridescenceThicknessMaximum=t.iridescenceThicknessMaximum;if(t.iridescenceThicknessRange!==void 0)this.iridescenceThicknessMinimum=t.iridescenceThicknessRange[0],this.iridescenceThicknessMaximum=t.iridescenceThicknessRange[1];if(t.transmission!==void 0)this.transmission=t.transmission;if(t.thickness!==void 0)this.thickness=t.thickness;if(t.attenuationDistance!==void 0)this.attenuationDistance=t.attenuationDistance;if(t.attenuationColor!==void 0&&this.attenuationColor!==void 0)this.attenuationColor.setHex(t.attenuationColor);if(t.anisotropy!==void 0)this.anisotropy=t.anisotropy;if(t.anisotropyRotation!==void 0)this.anisotropyRotation=t.anisotropyRotation;if(t.fog!==void 0)this.fog=t.fog;if(t.flatShading!==void 0)this.flatShading=t.flatShading;if(t.blending!==void 0)this.blending=t.blending;if(t.combine!==void 0)this.combine=t.combine;if(t.side!==void 0)this.side=t.side;if(t.shadowSide!==void 0)this.shadowSide=t.shadowSide;if(t.opacity!==void 0)this.opacity=t.opacity;if(t.transparent!==void 0)this.transparent=t.transparent;if(t.alphaTest!==void 0)this.alphaTest=t.alphaTest;if(t.alphaHash!==void 0)this.alphaHash=t.alphaHash;if(t.depthFunc!==void 0)this.depthFunc=t.depthFunc;if(t.depthTest!==void 0)this.depthTest=t.depthTest;if(t.depthWrite!==void 0)this.depthWrite=t.depthWrite;if(t.colorWrite!==void 0)this.colorWrite=t.colorWrite;if(t.clippingPlanes!==void 0)this.clippingPlanes=t.clippingPlanes.map((n)=>new Mn().fromJSON(n));if(t.clipIntersection!==void 0)this.clipIntersection=t.clipIntersection;if(t.clipShadows!==void 0)this.clipShadows=t.clipShadows;if(t.depthPacking!==void 0)this.depthPacking=t.depthPacking;if(t.blendSrc!==void 0)this.blendSrc=t.blendSrc;if(t.blendDst!==void 0)this.blendDst=t.blendDst;if(t.blendEquation!==void 0)this.blendEquation=t.blendEquation;if(t.blendSrcAlpha!==void 0)this.blendSrcAlpha=t.blendSrcAlpha;if(t.blendDstAlpha!==void 0)this.blendDstAlpha=t.blendDstAlpha;if(t.blendEquationAlpha!==void 0)this.blendEquationAlpha=t.blendEquationAlpha;if(t.blendColor!==void 0&&this.blendColor!==void 0)this.blendColor.setHex(t.blendColor);if(t.blendAlpha!==void 0)this.blendAlpha=t.blendAlpha;if(t.stencilWriteMask!==void 0)this.stencilWriteMask=t.stencilWriteMask;if(t.stencilFunc!==void 0)this.stencilFunc=t.stencilFunc;if(t.stencilRef!==void 0)this.stencilRef=t.stencilRef;if(t.stencilFuncMask!==void 0)this.stencilFuncMask=t.stencilFuncMask;if(t.stencilFail!==void 0)this.stencilFail=t.stencilFail;if(t.stencilZFail!==void 0)this.stencilZFail=t.stencilZFail;if(t.stencilZPass!==void 0)this.stencilZPass=t.stencilZPass;if(t.stencilWrite!==void 0)this.stencilWrite=t.stencilWrite;if(t.wireframe!==void 0)this.wireframe=t.wireframe;if(t.wireframeLinewidth!==void 0)this.wireframeLinewidth=t.wireframeLinewidth;if(t.wireframeLinecap!==void 0)this.wireframeLinecap=t.wireframeLinecap;if(t.wireframeLinejoin!==void 0)this.wireframeLinejoin=t.wireframeLinejoin;if(t.rotation!==void 0)this.rotation=t.rotation;if(t.linewidth!==void 0)this.linewidth=t.linewidth;if(t.linecap!==void 0)this.linecap=t.linecap;if(t.linejoin!==void 0)this.linejoin=t.linejoin;if(t.dashSize!==void 0)this.dashSize=t.dashSize;if(t.gapSize!==void 0)this.gapSize=t.gapSize;if(t.scale!==void 0)this.scale=t.scale;if(t.polygonOffset!==void 0)this.polygonOffset=t.polygonOffset;if(t.polygonOffsetFactor!==void 0)this.polygonOffsetFactor=t.polygonOffsetFactor;if(t.polygonOffsetUnits!==void 0)this.polygonOffsetUnits=t.polygonOffsetUnits;if(t.dithering!==void 0)this.dithering=t.dithering;if(t.alphaToCoverage!==void 0)this.alphaToCoverage=t.alphaToCoverage;if(t.premultipliedAlpha!==void 0)this.premultipliedAlpha=t.premultipliedAlpha;if(t.forceSinglePass!==void 0)this.forceSinglePass=t.forceSinglePass;if(t.allowOverride!==void 0)this.allowOverride=t.allowOverride;if(t.visible!==void 0)this.visible=t.visible;if(t.toneMapped!==void 0)this.toneMapped=t.toneMapped;if(t.userData!==void 0)this.userData=t.userData;if(t.vertexColors!==void 0)if(typeof t.vertexColors==="number")this.vertexColors=t.vertexColors>0;else this.vertexColors=t.vertexColors;if(t.size!==void 0)this.size=t.size;if(t.sizeAttenuation!==void 0)this.sizeAttenuation=t.sizeAttenuation;if(t.map!==void 0)this.map=e[t.map]||null;if(t.matcap!==void 0)this.matcap=e[t.matcap]||null;if(t.alphaMap!==void 0)this.alphaMap=e[t.alphaMap]||null;if(t.bumpMap!==void 0)this.bumpMap=e[t.bumpMap]||null;if(t.bumpScale!==void 0)this.bumpScale=t.bumpScale;if(t.normalMap!==void 0)this.normalMap=e[t.normalMap]||null;if(t.normalMapType!==void 0)this.normalMapType=t.normalMapType;if(t.normalScale!==void 0){let n=t.normalScale;if(Array.isArray(n)===!1)n=[n,n];this.normalScale=new fe().fromArray(n)}if(t.displacementMap!==void 0)this.displacementMap=e[t.displacementMap]||null;if(t.displacementScale!==void 0)this.displacementScale=t.displacementScale;if(t.displacementBias!==void 0)this.displacementBias=t.displacementBias;if(t.roughnessMap!==void 0)this.roughnessMap=e[t.roughnessMap]||null;if(t.metalnessMap!==void 0)this.metalnessMap=e[t.metalnessMap]||null;if(t.emissiveMap!==void 0)this.emissiveMap=e[t.emissiveMap]||null;if(t.emissiveIntensity!==void 0)this.emissiveIntensity=t.emissiveIntensity;if(t.specularMap!==void 0)this.specularMap=e[t.specularMap]||null;if(t.specularIntensityMap!==void 0)this.specularIntensityMap=e[t.specularIntensityMap]||null;if(t.specularColorMap!==void 0)this.specularColorMap=e[t.specularColorMap]||null;if(t.envMap!==void 0)this.envMap=e[t.envMap]||null;if(t.envMapRotation!==void 0)this.envMapRotation.fromArray(t.envMapRotation);if(t.envMapIntensity!==void 0)this.envMapIntensity=t.envMapIntensity;if(t.reflectivity!==void 0)this.reflectivity=t.reflectivity;if(t.refractionRatio!==void 0)this.refractionRatio=t.refractionRatio;if(t.lightMap!==void 0)this.lightMap=e[t.lightMap]||null;if(t.lightMapIntensity!==void 0)this.lightMapIntensity=t.lightMapIntensity;if(t.aoMap!==void 0)this.aoMap=e[t.aoMap]||null;if(t.aoMapIntensity!==void 0)this.aoMapIntensity=t.aoMapIntensity;if(t.gradientMap!==void 0)this.gradientMap=e[t.gradientMap]||null;if(t.clearcoatMap!==void 0)this.clearcoatMap=e[t.clearcoatMap]||null;if(t.clearcoatRoughnessMap!==void 0)this.clearcoatRoughnessMap=e[t.clearcoatRoughnessMap]||null;if(t.clearcoatNormalMap!==void 0)this.clearcoatNormalMap=e[t.clearcoatNormalMap]||null;if(t.clearcoatNormalScale!==void 0)this.clearcoatNormalScale=new fe().fromArray(t.clearcoatNormalScale);if(t.diffuseRoughnessMap!==void 0)this.diffuseRoughnessMap=e[t.diffuseRoughnessMap]||null;if(t.iridescenceMap!==void 0)this.iridescenceMap=e[t.iridescenceMap]||null;if(t.iridescenceThicknessMap!==void 0)this.iridescenceThicknessMap=e[t.iridescenceThicknessMap]||null;if(t.transmissionMap!==void 0)this.transmissionMap=e[t.transmissionMap]||null;if(t.thicknessMap!==void 0)this.thicknessMap=e[t.thicknessMap]||null;if(t.anisotropyMap!==void 0)this.anisotropyMap=e[t.anisotropyMap]||null;if(t.sheenColorMap!==void 0)this.sheenColorMap=e[t.sheenColorMap]||null;if(t.sheenRoughnessMap!==void 0)this.sheenRoughnessMap=e[t.sheenRoughnessMap]||null;return this}clone(){return new this.constructor().copy(this)}copy(t){this.name=t.name,this.blending=t.blending,this.side=t.side,this.vertexColors=t.vertexColors,this.opacity=t.opacity,this.transparent=t.transparent,this.blendSrc=t.blendSrc,this.blendDst=t.blendDst,this.blendEquation=t.blendEquation,this.blendSrcAlpha=t.blendSrcAlpha,this.blendDstAlpha=t.blendDstAlpha,this.blendEquationAlpha=t.blendEquationAlpha,this.blendColor.copy(t.blendColor),this.blendAlpha=t.blendAlpha,this.depthFunc=t.depthFunc,this.depthTest=t.depthTest,this.depthWrite=t.depthWrite,this.stencilWriteMask=t.stencilWriteMask,this.stencilFunc=t.stencilFunc,this.stencilRef=t.stencilRef,this.stencilFuncMask=t.stencilFuncMask,this.stencilFail=t.stencilFail,this.stencilZFail=t.stencilZFail,this.stencilZPass=t.stencilZPass,this.stencilWrite=t.stencilWrite;let e=t.clippingPlanes,n=null;if(e!==null){let i=e.length;n=Array(i);for(let s=0;s!==i;++s)n[s]=e[s].clone()}return this.clippingPlanes=n,this.clipIntersection=t.clipIntersection,this.clipShadows=t.clipShadows,this.shadowSide=t.shadowSide,this.colorWrite=t.colorWrite,this.precision=t.precision,this.polygonOffset=t.polygonOffset,this.polygonOffsetFactor=t.polygonOffsetFactor,this.polygonOffsetUnits=t.polygonOffsetUnits,this.dithering=t.dithering,this.alphaTest=t.alphaTest,this.alphaHash=t.alphaHash,this.alphaToCoverage=t.alphaToCoverage,this.premultipliedAlpha=t.premultipliedAlpha,this.forceSinglePass=t.forceSinglePass,this.allowOverride=t.allowOverride,this.visible=t.visible,this.toneMapped=t.toneMapped,this.userData=JSON.parse(JSON.stringify(t.userData)),this}dispose(){this.dispatchEvent({type:"dispose"})}set needsUpdate(t){if(t===!0)this.version++}}var Nn=new W,ea=new W,Ns=new W,Ds=new W;class us{constructor(t=new W,e=new W(0,0,-1)){this.origin=t,this.direction=e}set(t,e){return this.origin.copy(t),this.direction.copy(e),this}copy(t){return this.origin.copy(t.origin),this.direction.copy(t.direction),this}at(t,e){return e.copy(this.origin).addScaledVector(this.direction,t)}lookAt(t){return this.direction.copy(t).sub(this.origin).normalize(),this}recast(t){return this.origin.copy(this.at(t,Nn)),this}closestPointToPoint(t,e){e.subVectors(t,this.origin);let n=e.dot(this.direction);if(n<0)return e.copy(this.origin);return e.copy(this.origin).addScaledVector(this.direction,n)}distanceToPoint(t){return Math.sqrt(this.distanceSqToPoint(t))}distanceSqToPoint(t){let e=Nn.subVectors(t,this.origin).dot(this.direction);if(e<0)return this.origin.distanceToSquared(t);return Nn.copy(this.origin).addScaledVector(this.direction,e),Nn.distanceToSquared(t)}distanceSqToSegment(t,e,n,i){ea.copy(t).add(e).multiplyScalar(0.5),Ns.copy(e).sub(t).normalize(),Ds.copy(this.origin).sub(ea);let s=t.distanceTo(e)*0.5,r=-this.direction.dot(Ns),a=Ds.dot(this.direction),o=-Ds.dot(Ns),l=Ds.lengthSq(),c=Math.abs(1-r*r),p,h,u,d;if(c>0)if(p=r*o-a,h=r*a-o,d=s*c,p>=0)if(h>=-d)if(h<=d){let v=1/c;p*=v,h*=v,u=p*(p+r*h+2*a)+h*(r*p+h+2*o)+l}else h=s,p=Math.max(0,-(r*h+a)),u=-p*p+h*(h+2*o)+l;else h=-s,p=Math.max(0,-(r*h+a)),u=-p*p+h*(h+2*o)+l;else if(h<=-d)p=Math.max(0,-(-r*s+a)),h=p>0?-s:Math.min(Math.max(-s,-o),s),u=-p*p+h*(h+2*o)+l;else if(h<=d)p=0,h=Math.min(Math.max(-s,-o),s),u=h*(h+2*o)+l;else p=Math.max(0,-(r*s+a)),h=p>0?s:Math.min(Math.max(-s,-o),s),u=-p*p+h*(h+2*o)+l;else h=r>0?-s:s,p=Math.max(0,-(r*h+a)),u=-p*p+h*(h+2*o)+l;if(n)n.copy(this.origin).addScaledVector(this.direction,p);if(i)i.copy(ea).addScaledVector(Ns,h);return u}intersectSphere(t,e){if(t.radius<0)return null;Nn.subVectors(t.center,this.origin);let n=Nn.dot(this.direction),i=Nn.dot(Nn)-n*n,s=t.radius*t.radius;if(i>s)return null;let r=Math.sqrt(s-i),a=n-r,o=n+r;if(o<0)return null;if(a<0)return this.at(o,e);return this.at(a,e)}intersectsSphere(t){if(t.radius<0)return!1;return this.distanceSqToPoint(t.center)<=t.radius*t.radius}distanceToPlane(t){let e=t.normal.dot(this.direction);if(e===0){if(t.distanceToPoint(this.origin)===0)return 0;return null}let n=-(this.origin.dot(t.normal)+t.constant)/e;return n>=0?n:null}intersectPlane(t,e){let n=this.distanceToPlane(t);if(n===null)return null;return this.at(n,e)}intersectsPlane(t){let e=t.distanceToPoint(this.origin);if(e===0)return!0;if(t.normal.dot(this.direction)*e<0)return!0;return!1}intersectBox(t,e){let n,i,s,r,a,o,l=1/this.direction.x,c=1/this.direction.y,p=1/this.direction.z,h=this.origin;if(l>=0)n=(t.min.x-h.x)*l,i=(t.max.x-h.x)*l;else n=(t.max.x-h.x)*l,i=(t.min.x-h.x)*l;if(c>=0)s=(t.min.y-h.y)*c,r=(t.max.y-h.y)*c;else s=(t.max.y-h.y)*c,r=(t.min.y-h.y)*c;if(n>r||s>i)return null;if(s>n||isNaN(n))n=s;if(r<i||isNaN(i))i=r;if(p>=0)a=(t.min.z-h.z)*p,o=(t.max.z-h.z)*p;else a=(t.max.z-h.z)*p,o=(t.min.z-h.z)*p;if(n>o||a>i)return null;if(a>n||n!==n)n=a;if(o<i||i!==i)i=o;if(i<0)return null;return this.at(n>=0?n:i,e)}intersectsBox(t){return this.intersectBox(t,Nn)!==null}intersectTriangle(t,e,n,i,s){let r=this.origin,a=this.direction,{x:o,y:l,z:c}=a,p=t.x-r.x,h=t.y-r.y,u=t.z-r.z,d=e.x-r.x,v=e.y-r.y,w=e.z-r.z,f=n.x-r.x,m=n.y-r.y,M=n.z-r.z,T=Math.abs(o),S=Math.abs(l),E=Math.abs(c),P,g,y,D,q,N,X,rt,R,j,tt,G;if(T>=S&&T>=E)if(y=o,N=p,R=d,G=f,o>=0)P=l,g=c,D=h,q=u,X=v,rt=w,j=m,tt=M;else P=c,g=l,D=u,q=h,X=w,rt=v,j=M,tt=m;else if(S>=E)if(y=l,N=h,R=v,G=m,l>=0)P=c,g=o,D=u,q=p,X=w,rt=d,j=M,tt=f;else P=o,g=c,D=p,q=u,X=d,rt=w,j=f,tt=M;else if(y=c,N=u,R=w,G=M,c>=0)P=o,g=l,D=p,q=h,X=d,rt=v,j=f,tt=m;else P=l,g=o,D=h,q=p,X=v,rt=d,j=m,tt=f;if(y===0)return null;let pt=P/y,J=g/y,st=1/y,k=D-pt*N,It=q-J*N,nt=X-pt*R,ct=rt-J*R,yt=j-pt*G,Gt=tt-J*G,zt=yt*ct-Gt*nt,Dt=k*Gt-It*yt,ue=nt*It-ct*k;if(i){if(zt<0||Dt<0||ue<0)return null}else if((zt<0||Dt<0||ue<0)&&(zt>0||Dt>0||ue>0))return null;let ee=zt+Dt+ue;if(ee===0)return null;let Jt=st*(zt*N+Dt*R+ue*G);if(ee>0?Jt<0:Jt>0)return null;return this.at(Jt/ee,s)}applyMatrix4(t){return this.origin.applyMatrix4(t),this.direction.transformDirection(t),this}equals(t){return t.origin.equals(this.origin)&&t.direction.equals(this.direction)}clone(){return new this.constructor().copy(this)}}class uo extends $n{constructor(t){super();this.isMeshBasicMaterial=!0,this.type="MeshBasicMaterial",this.color=new le(16777215),this.map=null,this.lightMap=null,this.lightMapIntensity=1,this.aoMap=null,this.aoMapIntensity=1,this.specularMap=null,this.alphaMap=null,this.envMap=null,this.envMapRotation=new Xn,this.combine=0,this.reflectivity=1,this.refractionRatio=0.98,this.wireframe=!1,this.wireframeLinewidth=1,this.wireframeLinecap="round",this.wireframeLinejoin="round",this.fog=!0,this.setValues(t)}copy(t){return super.copy(t),this.color.copy(t.color),this.map=t.map,this.lightMap=t.lightMap,this.lightMapIntensity=t.lightMapIntensity,this.aoMap=t.aoMap,this.aoMapIntensity=t.aoMapIntensity,this.specularMap=t.specularMap,this.alphaMap=t.alphaMap,this.envMap=t.envMap,this.envMapRotation.copy(t.envMapRotation),this.combine=t.combine,this.reflectivity=t.reflectivity,this.refractionRatio=t.refractionRatio,this.wireframe=t.wireframe,this.wireframeLinewidth=t.wireframeLinewidth,this.wireframeLinecap=t.wireframeLinecap,this.wireframeLinejoin=t.wireframeLinejoin,this.fog=t.fog,this}}var al=new Se,si=new us,Us=new di,ol=new W,Fs=new W,Os=new W,Bs=new W,na=new W,zs=new W,ll=new W,ks=new W;class hn extends Xe{constructor(t=new Ee,e=new uo){super();this.isMesh=!0,this.type="Mesh",this.geometry=t,this.material=e,this.morphTargetDictionary=void 0,this.morphTargetInfluences=void 0,this.count=1,this.updateMorphTargets()}copy(t,e){if(super.copy(t,e),t.morphTargetInfluences!==void 0)this.morphTargetInfluences=t.morphTargetInfluences.slice();else this.morphTargetInfluences=void 0;if(t.morphTargetDictionary!==void 0)this.morphTargetDictionary=Object.assign({},t.morphTargetDictionary);else this.morphTargetDictionary=void 0;return this.material=Array.isArray(t.material)?t.material.slice():t.material,this.geometry=t.geometry,this}updateMorphTargets(){let e=this.geometry.morphAttributes,n=Object.keys(e);if(n.length>0){let i=e[n[0]];if(i!==void 0){this.morphTargetInfluences=[],this.morphTargetDictionary={};for(let s=0,r=i.length;s<r;s++){let a=i[s].name||String(s);this.morphTargetInfluences.push(0),this.morphTargetDictionary[a]=s}}}}getVertexPosition(t,e){let n=this.geometry,i=n.attributes.position,s=n.morphAttributes.position,r=n.morphTargetsRelative;e.fromBufferAttribute(i,t);let a=this.morphTargetInfluences;if(s&&a){zs.set(0,0,0);for(let o=0,l=s.length;o<l;o++){let c=a[o],p=s[o];if(c===0)continue;if(na.fromBufferAttribute(p,t),r)zs.addScaledVector(na,c);else zs.addScaledVector(na.sub(e),c)}e.add(zs)}return e}intersectsFrustum(t){return t.intersectsObject(this)}raycast(t,e){let n=this.geometry,i=this.material,s=this.matrixWorld;if(i===void 0)return;if(n.boundingSphere===null)n.computeBoundingSphere();if(Us.copy(n.boundingSphere),Us.applyMatrix4(s),si.copy(t.ray).recast(t.near),Us.containsPoint(si.origin)===!1){if(si.intersectSphere(Us,ol)===null)return;if(si.origin.distanceToSquared(ol)>(t.far-t.near)**2)return}if(al.copy(s).invert(),si.copy(t.ray).applyMatrix4(al),n.boundingBox!==null){if(si.intersectsBox(n.boundingBox)===!1)return}this._computeIntersections(t,e,si)}_computeIntersections(t,e,n){let i,s=this.geometry,r=this.material,a=s.index,o=s.attributes.position,l=s.attributes.uv,c=s.attributes.uv1,p=s.attributes.normal,{groups:h,drawRange:u}=s;if(a!==null)if(Array.isArray(r))for(let d=0,v=h.length;d<v;d++){let w=h[d],f=r[w.materialIndex],m=Math.max(w.start,u.start),M=Math.min(a.count,Math.min(w.start+w.count,u.start+u.count));for(let T=m,S=M;T<S;T+=3){let E=a.getX(T),P=a.getX(T+1),g=a.getX(T+2);if(i=Gs(this,f,t,n,l,c,p,E,P,g),i)i.faceIndex=Math.floor(T/3),i.face.materialIndex=w.materialIndex,e.push(i)}}else{let d=Math.max(0,u.start),v=Math.min(a.count,u.start+u.count);for(let w=d,f=v;w<f;w+=3){let m=a.getX(w),M=a.getX(w+1),T=a.getX(w+2);if(i=Gs(this,r,t,n,l,c,p,m,M,T),i)i.faceIndex=Math.floor(w/3),e.push(i)}}else if(o!==void 0)if(Array.isArray(r))for(let d=0,v=h.length;d<v;d++){let w=h[d],f=r[w.materialIndex],m=Math.max(w.start,u.start),M=Math.min(o.count,Math.min(w.start+w.count,u.start+u.count));for(let T=m,S=M;T<S;T+=3){let E=T,P=T+1,g=T+2;if(i=Gs(this,f,t,n,l,c,p,E,P,g),i)i.faceIndex=Math.floor(T/3),i.face.materialIndex=w.materialIndex,e.push(i)}}else{let d=Math.max(0,u.start),v=Math.min(o.count,u.start+u.count);for(let w=d,f=v;w<f;w+=3){let m=w,M=w+1,T=w+2;if(i=Gs(this,r,t,n,l,c,p,m,M,T),i)i.faceIndex=Math.floor(w/3),e.push(i)}}}}function qh(t,e,n,i,s,r,a,o){let l;if(e.side===1)l=i.intersectTriangle(a,r,s,!0,o);else l=i.intersectTriangle(s,r,a,e.side===0,o);if(l===null)return null;ks.copy(o),ks.applyMatrix4(t.matrixWorld);let c=n.ray.origin.distanceTo(ks);if(c<n.near||c>n.far)return null;return{distance:c,point:ks.clone(),object:t}}function Gs(t,e,n,i,s,r,a,o,l,c){t.getVertexPosition(o,Fs),t.getVertexPosition(l,Os),t.getVertexPosition(c,Bs);let p=qh(t,e,n,i,Fs,Os,Bs,ll);if(p){let h=new W;if(on.getBarycoord(ll,Fs,Os,Bs,h),s)p.uv=on.getInterpolatedAttribute(s,o,l,c,h,new fe);if(r)p.uv1=on.getInterpolatedAttribute(r,o,l,c,h,new fe);if(a){if(p.normal=on.getInterpolatedAttribute(a,o,l,c,h,new W),p.normal.dot(i.direction)>0)p.normal.multiplyScalar(-1)}let u={a:o,b:l,c,normal:new W,materialIndex:0};on.getNormal(Fs,Os,Bs,u.normal),p.face=u,p.barycoord=h}return p}class fo extends Ve{constructor(t=null,e=1,n=1,i,s,r,a,o,l=1003,c=1003,p,h){super(null,r,a,o,l,c,i,s,p,h);this.isDataTexture=!0,this.image={data:t,width:e,height:n},this.generateMipmaps=!1,this.flipY=!1,this.unpackAlignment=1}}class pr extends Ke{constructor(t,e,n,i=1){super(t,e,n);this.isInstancedBufferAttribute=!0,this.meshPerAttribute=i}copy(t){return super.copy(t),this.meshPerAttribute=t.meshPerAttribute,this}toJSON(){let t=super.toJSON();return t.meshPerAttribute=this.meshPerAttribute,t.isInstancedBufferAttribute=!0,t}}var ri=new di,Yh=new fe(0.5,0.5),Hs=new W;class Fi{constructor(t=new Mn,e=new Mn,n=new Mn,i=new Mn,s=new Mn,r=new Mn){this.planes=[t,e,n,i,s,r]}set(t,e,n,i,s,r){let a=this.planes;return a[0].copy(t),a[1].copy(e),a[2].copy(n),a[3].copy(i),a[4].copy(s),a[5].copy(r),this}copy(t){let e=this.planes;for(let n=0;n<6;n++)e[n].copy(t.planes[n]);return this}setFromProjectionMatrix(t,e=2000,n=!1){let i=this.planes,s=t.elements,r=s[0],a=s[1],o=s[2],l=s[3],c=s[4],p=s[5],h=s[6],u=s[7],d=s[8],v=s[9],w=s[10],f=s[11],m=s[12],M=s[13],T=s[14],S=s[15];if(i[0].setComponents(l-r,u-c,f-d,S-m).normalize(),i[1].setComponents(l+r,u+c,f+d,S+m).normalize(),i[2].setComponents(l+a,u+p,f+v,S+M).normalize(),i[3].setComponents(l-a,u-p,f-v,S-M).normalize(),n)i[4].setComponents(o,h,w,T).normalize(),i[5].setComponents(l-o,u-h,f-w,S-T).normalize();else if(i[4].setComponents(l-o,u-h,f-w,S-T).normalize(),e===2000)i[5].setComponents(l+o,u+h,f+w,S+T).normalize();else if(e===2001)i[5].setComponents(o,h,w,T).normalize();else throw Error("THREE.Frustum.setFromProjectionMatrix(): Invalid coordinate system: "+e);return this}intersectsObject(t){if(t.boundingSphere!==void 0){if(t.boundingSphere===null)t.computeBoundingSphere();ri.copy(t.boundingSphere).applyMatrix4(t.matrixWorld)}else{let e=t.geometry;if(e.boundingSphere===null)e.computeBoundingSphere();ri.copy(e.boundingSphere).applyMatrix4(t.matrixWorld)}return this.intersectsSphere(ri)}intersectsSprite(t){ri.center.set(0,0,0);let e=Yh.distanceTo(t.center);return ri.radius=0.7071067811865476+e,ri.applyMatrix4(t.matrixWorld),this.intersectsSphere(ri)}intersectsSphere(t){let e=this.planes,n=t.center,i=-t.radius;for(let s=0;s<6;s++)if(e[s].distanceToPoint(n)<i)return!1;return!0}intersectsBox(t){let e=this.planes;for(let n=0;n<6;n++){let i=e[n];if(Hs.x=i.normal.x>0?t.max.x:t.min.x,Hs.y=i.normal.y>0?t.max.y:t.min.y,Hs.z=i.normal.z>0?t.max.z:t.min.z,i.distanceToPoint(Hs)<0)return!1}return!0}containsPoint(t){let e=this.planes;for(let n=0;n<6;n++)if(e[n].distanceToPoint(t)<0)return!1;return!0}clone(){return new this.constructor().copy(this)}}var cl=new Se;class mr{constructor(){this.coordinateSystem=2000,this._frustums=[],this._count=0}setFromArrayCamera(t){let e=t.cameras,n=this._frustums;for(let i=0;i<e.length;i++){let s=e[i];if(cl.multiplyMatrices(s.projectionMatrix,s.matrixWorldInverse),n[i]===void 0)n[i]=new Fi;n[i].setFromProjectionMatrix(cl,s.coordinateSystem,s.reversedDepth)}return this._count=e.length,this}getFrustum(t){return this._frustums[t]}intersectsObject(t){let e=this._frustums;for(let n=0;n<this._count;n++)if(e[n].intersectsObject(t))return!0;return!1}intersectsSprite(t){let e=this._frustums;for(let n=0;n<this._count;n++)if(e[n].intersectsSprite(t))return!0;return!1}intersectsSphere(t){let e=this._frustums;for(let n=0;n<this._count;n++)if(e[n].intersectsSphere(t))return!0;return!1}intersectsBox(t){let e=this._frustums;for(let n=0;n<this._count;n++)if(e[n].intersectsBox(t))return!0;return!1}containsPoint(t){let e=this._frustums;for(let n=0;n<this._count;n++)if(e[n].containsPoint(t))return!0;return!1}copy(t){this.coordinateSystem=t.coordinateSystem;let e=this._frustums,n=t._frustums;for(let i=0;i<t._count;i++){if(e[i]===void 0)e[i]=new Fi;e[i].copy(n[i])}return this._count=t._count,this}clone(){return new mr().copy(this)}}class po extends $n{constructor(t){super();this.isLineBasicMaterial=!0,this.type="LineBasicMaterial",this.color=new le(16777215),this.map=null,this.linewidth=1,this.linecap="round",this.linejoin="round",this.fog=!0,this.setValues(t)}copy(t){return super.copy(t),this.color.copy(t.color),this.map=t.map,this.linewidth=t.linewidth,this.linecap=t.linecap,this.linejoin=t.linejoin,this.fog=t.fog,this}}var Ks=new W,Qs=new W,hl=new Se,ts=new us,Ws=new di,ia=new W,ul=new W;class mo extends Xe{constructor(t=new Ee,e=new po){super();this.isLine=!0,this.type="Line",this.geometry=t,this.material=e,this.morphTargetDictionary=void 0,this.morphTargetInfluences=void 0,this.updateMorphTargets()}copy(t,e){if(super.copy(t,e),t.morphTargetInfluences!==void 0)this.morphTargetInfluences=t.morphTargetInfluences.slice();else this.morphTargetInfluences=void 0;if(t.morphTargetDictionary!==void 0)this.morphTargetDictionary=Object.assign({},t.morphTargetDictionary);else this.morphTargetDictionary=void 0;return this.material=Array.isArray(t.material)?t.material.slice():t.material,this.geometry=t.geometry,this}computeLineDistances(){let t=this.geometry;if(t.index===null){let e=t.attributes.position,n=[0];for(let i=1,s=e.count;i<s;i++)Ks.fromBufferAttribute(e,i-1),Qs.fromBufferAttribute(e,i),n[i]=n[i-1],n[i]+=Ks.distanceTo(Qs);t.setAttribute("lineDistance",new sn(n,1))}else Wt("Line.computeLineDistances(): Computation only possible with non-indexed BufferGeometry.");return this}intersectsFrustum(t){return t.intersectsObject(this)}raycast(t,e){let n=this.geometry,i=this.matrixWorld,s=t.params.Line.threshold,r=n.drawRange;if(n.boundingSphere===null)n.computeBoundingSphere();if(Ws.copy(n.boundingSphere),Ws.applyMatrix4(i),Ws.radius+=s,t.ray.intersectsSphere(Ws)===!1)return;hl.copy(i).invert(),ts.copy(t.ray).applyMatrix4(hl);let a=s/((this.scale.x+this.scale.y+this.scale.z)/3),o=a*a,l=this.isLineSegments?2:1,c=n.index,h=n.attributes.position;if(c!==null){let u=Math.max(0,r.start),d=Math.min(c.count,r.start+r.count);for(let v=u,w=d-1;v<w;v+=l){let f=c.getX(v),m=c.getX(v+1),M=Vs(this,t,ts,o,f,m,v);if(M)e.push(M)}if(this.isLineLoop){let v=c.getX(d-1),w=c.getX(u),f=Vs(this,t,ts,o,v,w,d-1);if(f)e.push(f)}}else{let u=Math.max(0,r.start),d=Math.min(h.count,r.start+r.count);for(let v=u,w=d-1;v<w;v+=l){let f=Vs(this,t,ts,o,v,v+1,v);if(f)e.push(f)}if(this.isLineLoop){let v=Vs(this,t,ts,o,d-1,u,d-1);if(v)e.push(v)}}}updateMorphTargets(){let e=this.geometry.morphAttributes,n=Object.keys(e);if(n.length>0){let i=e[n[0]];if(i!==void 0){this.morphTargetInfluences=[],this.morphTargetDictionary={};for(let s=0,r=i.length;s<r;s++){let a=i[s].name||String(s);this.morphTargetInfluences.push(0),this.morphTargetDictionary[a]=s}}}}}function Vs(t,e,n,i,s,r,a){let o=t.geometry.attributes.position;if(Ks.fromBufferAttribute(o,s),Qs.fromBufferAttribute(o,r),n.distanceSqToSegment(Ks,Qs,ia,ul)>i)return;ia.applyMatrix4(t.matrixWorld);let c=e.ray.origin.distanceTo(ia);if(c<e.near||c>e.far)return;return{distance:c,point:ul.clone().applyMatrix4(t.matrixWorld),index:a,face:null,faceIndex:null,barycoord:null,object:t}}var dl=new W,fl=new W;class gr extends mo{constructor(t,e){super(t,e);this.isLineSegments=!0,this.type="LineSegments"}computeLineDistances(){let t=this.geometry;if(t.index===null){let e=t.attributes.position,n=[];for(let i=0,s=e.count;i<s;i+=2)dl.fromBufferAttribute(e,i),fl.fromBufferAttribute(e,i+1),n[i]=i===0?0:n[i-1],n[i+1]=n[i]+dl.distanceTo(fl);t.setAttribute("lineDistance",new sn(n,1))}else Wt("LineSegments.computeLineDistances(): Computation only possible with non-indexed BufferGeometry.");return this}}class go extends $n{constructor(t){super();this.isPointsMaterial=!0,this.type="PointsMaterial",this.color=new le(16777215),this.map=null,this.alphaMap=null,this.size=1,this.sizeAttenuation=!0,this.fog=!0,this.setValues(t)}copy(t){return super.copy(t),this.color.copy(t.color),this.map=t.map,this.alphaMap=t.alphaMap,this.size=t.size,this.sizeAttenuation=t.sizeAttenuation,this.fog=t.fog,this}}var pl=new Se,sa=new us,Xs=new di,qs=new W;class Hi extends Xe{constructor(t=new Ee,e=new go){super();this.isPoints=!0,this.type="Points",this.geometry=t,this.material=e,this.morphTargetDictionary=void 0,this.morphTargetInfluences=void 0,this.updateMorphTargets()}copy(t,e){if(super.copy(t,e),t.morphTargetInfluences!==void 0)this.morphTargetInfluences=t.morphTargetInfluences.slice();else this.morphTargetInfluences=void 0;if(t.morphTargetDictionary!==void 0)this.morphTargetDictionary=Object.assign({},t.morphTargetDictionary);else this.morphTargetDictionary=void 0;return this.material=Array.isArray(t.material)?t.material.slice():t.material,this.geometry=t.geometry,this}intersectsFrustum(t){return t.intersectsObject(this)}raycast(t,e){let n=this.geometry,i=this.matrixWorld,s=t.params.Points.threshold,r=n.drawRange;if(n.boundingSphere===null)n.computeBoundingSphere();if(Xs.copy(n.boundingSphere),Xs.applyMatrix4(i),Xs.radius+=s,t.ray.intersectsSphere(Xs)===!1)return;pl.copy(i).invert(),sa.copy(t.ray).applyMatrix4(pl);let a=s/((this.scale.x+this.scale.y+this.scale.z)/3),o=a*a,l=n.index,p=n.attributes.position;if(l!==null){let h=Math.max(0,r.start),u=Math.min(l.count,r.start+r.count);for(let d=h,v=u;d<v;d++){let w=l.getX(d);qs.fromBufferAttribute(p,w),ml(qs,w,o,i,t,e,this)}}else{let h=Math.max(0,r.start),u=Math.min(p.count,r.start+r.count);for(let d=h,v=u;d<v;d++)qs.fromBufferAttribute(p,d),ml(qs,d,o,i,t,e,this)}}updateMorphTargets(){let e=this.geometry.morphAttributes,n=Object.keys(e);if(n.length>0){let i=e[n[0]];if(i!==void 0){this.morphTargetInfluences=[],this.morphTargetDictionary={};for(let s=0,r=i.length;s<r;s++){let a=i[s].name||String(s);this.morphTargetInfluences.push(0),this.morphTargetDictionary[a]=s}}}}}function ml(t,e,n,i,s,r,a){let o=sa.distanceSqToPoint(t);if(o<n){let l=new W;sa.closestPointToPoint(t,l),l.applyMatrix4(i);let c=s.ray.origin.distanceTo(l);if(c<s.near||c>s.far)return;r.push({distance:c,distanceToRay:Math.sqrt(o),point:l,index:e,face:null,faceIndex:null,barycoord:null,object:a})}}class _r extends Ve{constructor(t=[],e=301,n,i,s,r,a,o,l,c){super(t,e,n,i,s,r,a,o,l,c);this.isCubeTexture=!0,this.flipY=!1}get images(){return this.image}set images(t){this.image=t}}class Kn extends Ve{constructor(t,e,n=1014,i,s,r,a=1003,o=1003,l,c=1026,p=1){if(c!==1026&&c!==1027)throw Error("THREE.DepthTexture: format must be either THREE.DepthFormat or THREE.DepthStencilFormat");let h={width:t,height:e,depth:p};super(h,i,s,r,a,o,c,n,l);this.isDepthTexture=!0,this.flipY=!1,this.generateMipmaps=!1,this.compareFunction=null}copy(t){return super.copy(t),this.source=new cs(Object.assign({},t.image)),this.compareFunction=t.compareFunction,this}toJSON(t){let e=super.toJSON(t);return e.compareFunction=this.compareFunction,e}}class xr extends Kn{constructor(t,e=1014,n=301,i,s,r=1003,a=1003,o,l=1026){let c={width:t,height:t,depth:1},p=[c,c,c,c,c,c];super(t,t,e,n,i,s,r,a,o,l);this.image=p,this.isCubeDepthTexture=!0,this.isCubeTexture=!0}get images(){return this.image}set images(t){this.image=t}}class vr extends Ve{constructor(t=null){super();this.sourceTexture=t,this.isExternalTexture=!0}copy(t){return super.copy(t),this.sourceTexture=t.sourceTexture,this}}class Wi extends Ee{constructor(t=1,e=1,n=1,i=1,s=1,r=1){super();this.type="BoxGeometry",this.parameters={width:t,height:e,depth:n,widthSegments:i,heightSegments:s,depthSegments:r};let a=this;i=Math.floor(i),s=Math.floor(s),r=Math.floor(r);let o=[],l=[],c=[],p=[],h=0,u=0;d("z","y","x",-1,-1,n,e,t,r,s,0),d("z","y","x",1,-1,n,e,-t,r,s,1),d("x","z","y",1,1,t,n,e,i,r,2),d("x","z","y",1,-1,t,n,-e,i,r,3),d("x","y","z",1,-1,t,e,n,i,s,4),d("x","y","z",-1,-1,t,e,-n,i,s,5),this.setIndex(o),this.setAttribute("position",new sn(l,3)),this.setAttribute("normal",new sn(c,3)),this.setAttribute("uv",new sn(p,2));function d(v,w,f,m,M,T,S,E,P,g,y){let D=T/P,q=S/g,N=T/2,X=S/2,rt=E/2,R=P+1,j=g+1,tt=0,G=0,pt=new W;for(let J=0;J<j;J++){let st=J*q-X;for(let k=0;k<R;k++){let It=k*D-N;pt[v]=It*m,pt[w]=st*M,pt[f]=rt,l.push(pt.x,pt.y,pt.z),pt[v]=0,pt[w]=0,pt[f]=E>0?1:-1,c.push(pt.x,pt.y,pt.z),p.push(k/P),p.push(1-J/g),tt+=1}}for(let J=0;J<g;J++)for(let st=0;st<P;st++){let k=h+st+R*J,It=h+st+R*(J+1),nt=h+(st+1)+R*(J+1),ct=h+(st+1)+R*J;o.push(k,It,ct),o.push(It,nt,ct),G+=6}a.addGroup(u,G,y),u+=G,h+=tt}}copy(t){return super.copy(t),this.parameters=Object.assign({},t.parameters),this}static fromJSON(t){return new Wi(t.width,t.height,t.depth,t.widthSegments,t.heightSegments,t.depthSegments)}}class ds extends Ee{constructor(t=1,e=1,n=1,i=1){super();this.type="PlaneGeometry",this.parameters={width:t,height:e,widthSegments:n,heightSegments:i};let s=t/2,r=e/2,a=Math.floor(n),o=Math.floor(i),l=a+1,c=o+1,p=t/a,h=e/o,u=[],d=[],v=[],w=[];for(let f=0;f<c;f++){let m=f*h-r;for(let M=0;M<l;M++){let T=M*p-s;d.push(T,-m,0),v.push(0,0,1),w.push(M/a),w.push(1-f/o)}}for(let f=0;f<o;f++)for(let m=0;m<a;m++){let M=m+l*f,T=m+l*(f+1),S=m+1+l*(f+1),E=m+1+l*f;u.push(M,T,E),u.push(T,S,E)}this.setIndex(u),this.setAttribute("position",new sn(d,3)),this.setAttribute("normal",new sn(v,3)),this.setAttribute("uv",new sn(w,2))}copy(t){return super.copy(t),this.parameters=Object.assign({},t.parameters),this}static fromJSON(t){return new ds(t.width,t.height,t.widthSegments,t.heightSegments)}}function fi(t){let e={};for(let n in t){e[n]={};for(let i in t[n]){let s=t[n][i];if(gl(s))if(s.isRenderTargetTexture)Wt("UniformsUtils: Textures of render targets cannot be cloned via cloneUniforms() or mergeUniforms()."),e[n][i]=null;else e[n][i]=s.clone();else if(Array.isArray(s))if(gl(s[0])){let r=[];for(let a=0,o=s.length;a<o;a++)r[a]=s[a].clone();e[n][i]=r}else e[n][i]=s.slice();else e[n][i]=s}}return e}function qe(t){let e={};for(let n=0;n<t.length;n++){let i=fi(t[n]);for(let s in i)e[s]=i[s]}return e}function gl(t){return t&&(t.isColor||t.isMatrix3||t.isMatrix4||t.isVector2||t.isVector3||t.isVector4||t.isTexture||t.isQuaternion)}function Zh(t){let e=[];for(let n=0;n<t.length;n++)e.push(t[n].clone());return e}function _o(t){let e=t.getRenderTarget();if(e===null)return t.outputColorSpace;if(e.isXRRenderTarget===!0)return e.texture.colorSpace;return ae.workingColorSpace}var wc={clone:fi,merge:qe},Jh=`void main() {
	gl_Position = projectionMatrix * modelViewMatrix * vec4( position, 1.0 );
}`,$h=`void main() {
	gl_FragColor = vec4( 1.0, 0.0, 0.0, 1.0 );
}`;class vn extends $n{constructor(t){super();if(this.isShaderMaterial=!0,this.type="ShaderMaterial",this.defines={},this.uniforms={},this.uniformsGroups=[],this.vertexShader=Jh,this.fragmentShader=$h,this.linewidth=1,this.wireframe=!1,this.wireframeLinewidth=1,this.fog=!1,this.lights=!1,this.clipping=!1,this.forceSinglePass=!0,this.extensions={clipCullDistance:!1,multiDraw:!1},this.defaultAttributeValues={color:[1,1,1],uv:[0,0],uv1:[0,0]},this.index0AttributeName=void 0,this.uniformsNeedUpdate=!1,this.glslVersion=null,t!==void 0)this.setValues(t)}copy(t){return super.copy(t),this.fragmentShader=t.fragmentShader,this.vertexShader=t.vertexShader,this.uniforms=fi(t.uniforms),this.uniformsGroups=Zh(t.uniformsGroups),this.defines=Object.assign({},t.defines),this.wireframe=t.wireframe,this.wireframeLinewidth=t.wireframeLinewidth,this.fog=t.fog,this.lights=t.lights,this.clipping=t.clipping,this.extensions=Object.assign({},t.extensions),this.glslVersion=t.glslVersion,this.defaultAttributeValues=Object.assign({},t.defaultAttributeValues),this.index0AttributeName=t.index0AttributeName,this.uniformsNeedUpdate=t.uniformsNeedUpdate,this}toJSON(t){let e=super.toJSON(t);e.glslVersion=this.glslVersion,e.uniforms={};for(let i in this.uniforms){let r=this.uniforms[i].value;if(r&&r.isTexture)e.uniforms[i]={type:"t",value:r.toJSON(t).uuid};else if(r&&r.isColor)e.uniforms[i]={type:"c",value:r.getHex()};else if(r&&r.isVector2)e.uniforms[i]={type:"v2",value:r.toArray()};else if(r&&r.isVector3)e.uniforms[i]={type:"v3",value:r.toArray()};else if(r&&r.isVector4)e.uniforms[i]={type:"v4",value:r.toArray()};else if(r&&r.isMatrix3)e.uniforms[i]={type:"m3",value:r.toArray()};else if(r&&r.isMatrix4)e.uniforms[i]={type:"m4",value:r.toArray()};else e.uniforms[i]={value:r}}if(Object.keys(this.defines).length>0)e.defines=this.defines;e.vertexShader=this.vertexShader,e.fragmentShader=this.fragmentShader,e.lights=this.lights,e.clipping=this.clipping;let n={};for(let i in this.extensions)if(this.extensions[i]===!0)n[i]=!0;if(Object.keys(n).length>0)e.extensions=n;return e}fromJSON(t,e){if(super.fromJSON(t,e),t.uniforms!==void 0)for(let n in t.uniforms){let i=t.uniforms[n];switch(this.uniforms[n]={},i.type){case"t":this.uniforms[n].value=e[i.value]||null;break;case"c":this.uniforms[n].value=new le().setHex(i.value);break;case"v2":this.uniforms[n].value=new fe().fromArray(i.value);break;case"v3":this.uniforms[n].value=new W().fromArray(i.value);break;case"v4":this.uniforms[n].value=new Ae().fromArray(i.value);break;case"m3":this.uniforms[n].value=new Zt().fromArray(i.value);break;case"m4":this.uniforms[n].value=new Se().fromArray(i.value);break;default:this.uniforms[n].value=i.value}}if(t.defines!==void 0)this.defines=t.defines;if(t.vertexShader!==void 0)this.vertexShader=t.vertexShader;if(t.fragmentShader!==void 0)this.fragmentShader=t.fragmentShader;if(t.glslVersion!==void 0)this.glslVersion=t.glslVersion;if(t.extensions!==void 0)for(let n in t.extensions)this.extensions[n]=t.extensions[n];if(t.lights!==void 0)this.lights=t.lights;if(t.clipping!==void 0)this.clipping=t.clipping;return this}}class xo extends vn{constructor(t){super(t);this.isRawShaderMaterial=!0,this.type="RawShaderMaterial"}}class vo extends $n{constructor(t){super();this.isMeshDepthMaterial=!0,this.type="MeshDepthMaterial",this.depthPacking=3200,this.map=null,this.alphaMap=null,this.displacementMap=null,this.displacementScale=1,this.displacementBias=0,this.wireframe=!1,this.wireframeLinewidth=1,this.setValues(t)}copy(t){return super.copy(t),this.depthPacking=t.depthPacking,this.map=t.map,this.alphaMap=t.alphaMap,this.displacementMap=t.displacementMap,this.displacementScale=t.displacementScale,this.displacementBias=t.displacementBias,this.wireframe=t.wireframe,this.wireframeLinewidth=t.wireframeLinewidth,this}}class yo extends $n{constructor(t){super();this.isMeshDistanceMaterial=!0,this.type="MeshDistanceMaterial",this.map=null,this.alphaMap=null,this.displacementMap=null,this.displacementScale=1,this.displacementBias=0,this.setValues(t)}copy(t){return super.copy(t),this.map=t.map,this.alphaMap=t.alphaMap,this.displacementMap=t.displacementMap,this.displacementScale=t.displacementScale,this.displacementBias=t.displacementBias,this}}function Ii(t,e){if(!t||t.constructor===e)return t;if(typeof e.BYTES_PER_ELEMENT==="number")return new e(t);return Array.prototype.slice.call(t)}function Ys(t){return t!==void 0&&t.inTangents!==void 0&&t.outTangents!==void 0}class pi{constructor(t,e,n,i){this.parameterPositions=t,this._cachedIndex=0,this.resultBuffer=i!==void 0?i:new e.constructor(n),this.sampleValues=e,this.valueSize=n,this.settings=null,this.DefaultSettings_={}}evaluate(t){let e=this.parameterPositions,n=this._cachedIndex,i=e[n],s=e[n-1];n:{t:{let r;e:{i:if(!(t<i)){for(let a=n+2;;){if(i===void 0){if(t<s)break i;return n=e.length,this._cachedIndex=n,this.copySampleValue_(n-1)}if(n===a)break;if(s=i,i=e[++n],t<i)break t}r=e.length;break e}if(!(t>=s)){let a=e[1];if(t<a)n=2,s=a;for(let o=n-2;;){if(s===void 0)return this._cachedIndex=0,this.copySampleValue_(0);if(n===o)break;if(i=s,s=e[--n-1],t>=s)break t}r=n,n=0;break e}break n}while(n<r){let a=n+r>>>1;if(t<e[a])r=a;else n=a+1}if(i=e[n],s=e[n-1],s===void 0)return this._cachedIndex=0,this.copySampleValue_(0);if(i===void 0)return n=e.length,this._cachedIndex=n,this.copySampleValue_(n-1)}this._cachedIndex=n,this.intervalChanged_(n,s,i)}return this.interpolate_(n,s,t,i)}getSettings_(){return this.settings||this.DefaultSettings_}copySampleValue_(t){let e=this.resultBuffer,n=this.sampleValues,i=this.valueSize,s=t*i;for(let r=0;r!==i;++r)e[r]=n[s+r];return e}interpolate_(){throw Error("THREE.Interpolant: Call to abstract method.")}intervalChanged_(){}}class Mo extends pi{constructor(t,e,n,i){super(t,e,n,i);this._weightPrev=-0,this._offsetPrev=-0,this._weightNext=-0,this._offsetNext=-0,this.DefaultSettings_={endingStart:2400,endingEnd:2400}}intervalChanged_(t,e,n){let i=this.parameterPositions,s=t-2,r=t+1,a=i[s],o=i[r];if(a===void 0)switch(this.getSettings_().endingStart){case 2401:s=t,a=2*e-n;break;case 2402:s=i.length-2,a=e+i[s]-i[s+1];break;default:s=t,a=n}if(o===void 0)switch(this.getSettings_().endingEnd){case 2401:r=t,o=2*n-e;break;case 2402:r=1,o=n+i[1]-i[0];break;default:r=t-1,o=e}let l=(n-e)*0.5,c=this.valueSize;this._weightPrev=l/(e-a),this._weightNext=l/(o-n),this._offsetPrev=s*c,this._offsetNext=r*c}interpolate_(t,e,n,i){let s=this.resultBuffer,r=this.sampleValues,a=this.valueSize,o=t*a,l=o-a,c=this._offsetPrev,p=this._offsetNext,h=this._weightPrev,u=this._weightNext,d=(n-e)/(i-e),v=d*d,w=v*d,f=-h*w+2*h*v-h*d,m=(1+h)*w+(-1.5-2*h)*v+(-0.5+h)*d+1,M=(-1-u)*w+(1.5+u)*v+0.5*d,T=u*w-u*v;for(let S=0;S!==a;++S)s[S]=f*r[c+S]+m*r[l+S]+M*r[o+S]+T*r[p+S];return s}}class So extends pi{constructor(t,e,n,i){super(t,e,n,i)}interpolate_(t,e,n,i){let s=this.resultBuffer,r=this.sampleValues,a=this.valueSize,o=t*a,l=o-a,c=(n-e)/(i-e),p=1-c;for(let h=0;h!==a;++h)s[h]=r[l+h]*p+r[o+h]*c;return s}}class bo extends pi{constructor(t,e,n,i){super(t,e,n,i)}interpolate_(t){return this.copySampleValue_(t-1)}}class wo extends pi{interpolate_(t,e,n,i){let s=this.resultBuffer,r=this.sampleValues,a=this.valueSize,o=t*a,l=o-a,c=this.inTangents,p=this.outTangents;if(!c||!p){let d=(n-e)/(i-e),v=1-d;for(let w=0;w!==a;++w)s[w]=r[l+w]*v+r[o+w]*d;return s}let h=a*2,u=t-1;for(let d=0;d!==a;++d){let v=r[l+d],w=r[o+d],f=u*h+d*2,m=p[f],M=p[f+1],T=t*h+d*2,S=c[T],E=c[T+1],P=Qh(n,e,m,S,i);s[d]=Ac(P,v,M,E,w)}return s}}function Ac(t,e,n,i,s){let r=1-t;return r*r*r*e+3*r*r*t*n+3*r*t*t*i+t*t*t*s}function Kh(t,e,n,i,s){let r=1-t;return 3*r*r*(n-e)+6*r*t*(i-n)+3*t*t*(s-i)}function Qh(t,e,n,i,s){let r=(t-e)/(s-e);for(let a=0;a<8;a++){let o=Ac(r,e,n,i,s)-t;if(Math.abs(o)<0.0000000001)break;let l=Kh(r,e,n,i,s);if(Math.abs(l)<0.0000000001)break;r=Math.max(0,Math.min(1,r-o/l))}return r}class un{constructor(t,e,n,i){if(t===void 0)throw Error("THREE.KeyframeTrack: track name is undefined");if(e===void 0||e.length===0)throw Error("THREE.KeyframeTrack: no keyframes in track named "+t);this.name=t,this.times=Ii(e,this.TimeBufferType),this.values=Ii(n,this.ValueBufferType),this.setInterpolation(i||this.DefaultInterpolation)}static toJSON(t){let e=t.constructor,n;if(e.toJSON!==this.toJSON)n=e.toJSON(t);else{n={name:t.name,times:Ii(t.times,Array),values:Ii(t.values,Array)};let i=t.getInterpolation();if(i!==t.DefaultInterpolation)n.interpolation=i;if(Ys(t.settings))n.settings={inTangents:Ii(t.settings.inTangents,Array),outTangents:Ii(t.settings.outTangents,Array)}}return n.type=t.ValueTypeName,n}InterpolantFactoryMethodDiscrete(t){return new bo(this.times,this.values,this.getValueSize(),t)}InterpolantFactoryMethodLinear(t){return new So(this.times,this.values,this.getValueSize(),t)}InterpolantFactoryMethodSmooth(t){return new Mo(this.times,this.values,this.getValueSize(),t)}InterpolantFactoryMethodBezier(t){let e=new wo(this.times,this.values,this.getValueSize(),t);if(this.settings)e.inTangents=this.settings.inTangents,e.outTangents=this.settings.outTangents;return e}setInterpolation(t){let e;switch(t){case 2300:e=this.InterpolantFactoryMethodDiscrete;break;case 2301:e=this.InterpolantFactoryMethodLinear;break;case 2302:e=this.InterpolantFactoryMethodSmooth;break;case 2303:e=this.InterpolantFactoryMethodBezier;break}if(e===void 0){let n="unsupported interpolation for "+this.ValueTypeName+" keyframe track named "+this.name;if(this.createInterpolant===void 0)if(t!==this.DefaultInterpolation)this.setInterpolation(this.DefaultInterpolation);else throw Error(n);return Wt("KeyframeTrack:",n),this}return this.createInterpolant=e,this}getInterpolation(){switch(this.createInterpolant){case this.InterpolantFactoryMethodDiscrete:return 2300;case this.InterpolantFactoryMethodLinear:return 2301;case this.InterpolantFactoryMethodSmooth:return 2302;case this.InterpolantFactoryMethodBezier:return 2303}}getValueSize(){return this.values.length/this.times.length}shift(t){if(t!==0){let e=this.times;for(let n=0,i=e.length;n!==i;++n)e[n]+=t;if(Ys(this.settings))_l(this.settings.inTangents,t),_l(this.settings.outTangents,t)}return this}scale(t){if(t!==1){let e=this.times;for(let n=0,i=e.length;n!==i;++n)e[n]*=t;if(Ys(this.settings))xl(this.settings.inTangents,t),xl(this.settings.outTangents,t)}return this}trim(t,e){let n=this.times,i=n.length,s=0,r=i-1;while(s!==i&&n[s]<t)++s;while(r!==-1&&n[r]>e)--r;if(++r,s!==0||r!==i){if(s>=r)r=Math.max(r,1),s=r-1;let a=this.getValueSize();this.times=n.slice(s,r),this.values=this.values.slice(s*a,r*a)}return this}validate(){let t=!0,e=this.getValueSize();if(e-Math.floor(e)!==0)Yt("KeyframeTrack: Invalid value size in track.",this),t=!1;let n=this.times,i=this.values,s=n.length;if(s===0)Yt("KeyframeTrack: Track is empty.",this),t=!1;let r=null;for(let a=0;a!==s;a++){let o=n[a];if(typeof o==="number"&&isNaN(o)){Yt("KeyframeTrack: Time is not a valid number.",this,a,o),t=!1;break}if(r!==null&&r>o){Yt("KeyframeTrack: Out of order keys.",this,a,o,r),t=!1;break}r=o}if(i!==void 0){if(Th(i))for(let a=0,o=i.length;a!==o;++a){let l=i[a];if(isNaN(l)){Yt("KeyframeTrack: Value is not a valid number.",this,a,l),t=!1;break}}}return t}optimize(){let t=this.times.slice(),e=this.values.slice(),n=this.getValueSize(),i=this.getInterpolation()===2302,s=t.length-1,r=1;for(let a=1;a<s;++a){let o=!1,l=t[a],c=t[a+1];if(l!==c&&(a!==1||l!==t[0]))if(!i){let p=a*n,h=p-n,u=p+n;for(let d=0;d!==n;++d){let v=e[p+d];if(v!==e[h+d]||v!==e[u+d]){o=!0;break}}}else o=!0;if(o){if(a!==r){t[r]=t[a];let p=a*n,h=r*n;for(let u=0;u!==n;++u)e[h+u]=e[p+u]}++r}}if(s>0){t[r]=t[s];for(let a=s*n,o=r*n,l=0;l!==n;++l)e[o+l]=e[a+l];++r}if(r!==t.length)this.times=t.slice(0,r),this.values=e.slice(0,r*n);else this.times=t,this.values=e;return this}clone(){let t=this.times.slice(),e=this.values.slice(),i=new this.constructor(this.name,t,e);if(i.createInterpolant=this.createInterpolant,Ys(this.settings))i.settings={inTangents:this.settings.inTangents.slice(),outTangents:this.settings.outTangents.slice()};return i}}function _l(t,e){for(let n=0,i=t.length;n!==i;n+=2)t[n]+=e}function xl(t,e){for(let n=0,i=t.length;n!==i;n+=2)t[n]*=e}un.prototype.ValueTypeName="";un.prototype.TimeBufferType=Float32Array;un.prototype.ValueBufferType=Float32Array;un.prototype.DefaultInterpolation=2301;class mi extends un{constructor(t,e,n){super(t,e,n)}}mi.prototype.ValueTypeName="bool";mi.prototype.ValueBufferType=Array;mi.prototype.DefaultInterpolation=2300;mi.prototype.InterpolantFactoryMethodLinear=void 0;mi.prototype.InterpolantFactoryMethodSmooth=void 0;class Ao extends un{constructor(t,e,n,i){super(t,e,n,i)}}Ao.prototype.ValueTypeName="color";class To extends un{constructor(t,e,n,i){super(t,e,n,i)}}To.prototype.ValueTypeName="number";class Eo extends pi{constructor(t,e,n,i){super(t,e,n,i)}interpolate_(t,e,n,i){let s=this.resultBuffer,r=this.sampleValues,a=this.valueSize,o=(n-e)/(i-e),l=t*a;for(let c=l+a;l!==c;l+=4)Bn.slerpFlat(s,0,r,l-a,r,l,o);return s}}class yr extends un{constructor(t,e,n,i){super(t,e,n,i)}InterpolantFactoryMethodLinear(t){return new Eo(this.times,this.values,this.getValueSize(),t)}}yr.prototype.ValueTypeName="quaternion";yr.prototype.InterpolantFactoryMethodSmooth=void 0;class gi extends un{constructor(t,e,n){super(t,e,n)}}gi.prototype.ValueTypeName="string";gi.prototype.ValueBufferType=Array;gi.prototype.DefaultInterpolation=2300;gi.prototype.InterpolantFactoryMethodLinear=void 0;gi.prototype.InterpolantFactoryMethodSmooth=void 0;class Ro extends un{constructor(t,e,n,i){super(t,e,n,i)}}Ro.prototype.ValueTypeName="vector";class Co{constructor(t,e,n){let i=this,s=!1,r=0,a=0,o=void 0,l=[];this.onStart=void 0,this.onLoad=t,this.onProgress=e,this.onError=n,this._abortController=null,this.itemStart=function(c){if(a++,s===!1){if(i.onStart!==void 0)i.onStart(c,r,a)}s=!0},this.itemEnd=function(c){if(r++,i.onProgress!==void 0)i.onProgress(c,r,a);if(r===a){if(s=!1,i.onLoad!==void 0)i.onLoad()}},this.itemError=function(c){if(i.onError!==void 0)i.onError(c)},this.resolveURL=function(c){if(c=c.normalize("NFC"),o)return o(c);return c},this.setURLModifier=function(c){return o=c,this},this.addHandler=function(c,p){return l.push(c,p),this},this.removeHandler=function(c){let p=l.indexOf(c);if(p!==-1)l.splice(p,2);return this},this.getHandler=function(c){for(let p=0,h=l.length;p<h;p+=2){let u=l[p],d=l[p+1];if(u.global)u.lastIndex=0;if(u.test(c))return d}return null},this.abort=function(){return this.abortController.abort(),this._abortController=null,this}}get abortController(){if(!this._abortController)this._abortController=new AbortController;return this._abortController}}var Tc=new Co;class Io{constructor(t){if(this.manager=t!==void 0?t:Tc,this.crossOrigin="anonymous",this.withCredentials=!1,this.path="",this.resourcePath="",this.requestHeader={},this.priority="auto",this.cache="default",typeof __THREE_DEVTOOLS__<"u")__THREE_DEVTOOLS__.dispatchEvent(new CustomEvent("observe",{detail:this}))}load(){}loadAsync(t,e){let n=this;return new Promise(function(i,s){n.load(t,i,e,s)})}parse(){}setCrossOrigin(t){return this.crossOrigin=t,this}setWithCredentials(t){return this.withCredentials=t,this}setPath(t){return this.path=t,this}setResourcePath(t){return this.resourcePath=t,this}setRequestHeader(t){return this.requestHeader=t,this}setPriority(t){return this.priority=t,this}setCache(t){return this.cache=t,this}abort(){return this}}Io.DEFAULT_MATERIAL_NAME="__DEFAULT";var Zs=new W,Js=new Bn,yn=new W;class Mr extends Xe{constructor(){super();this.isCamera=!0,this.type="Camera",this.matrixWorldInverse=new Se,this.projectionMatrix=new Se,this.projectionMatrixInverse=new Se,this.coordinateSystem=2000,this._reversedDepth=!1}get reversedDepth(){return this._reversedDepth}copy(t,e){return super.copy(t,e),this.matrixWorldInverse.copy(t.matrixWorldInverse),this.projectionMatrix.copy(t.projectionMatrix),this.projectionMatrixInverse.copy(t.projectionMatrixInverse),this.coordinateSystem=t.coordinateSystem,this}getWorldDirection(t){return super.getWorldDirection(t).negate()}updateMatrixWorld(t){if(super.updateMatrixWorld(t),this.matrixWorld.decompose(Zs,Js,yn),yn.x===1&&yn.y===1&&yn.z===1)this.matrixWorldInverse.copy(this.matrixWorld).invert();else this.matrixWorldInverse.compose(Zs,Js,yn.set(1,1,1)).invert()}updateWorldMatrix(t,e,n=!1){if(super.updateWorldMatrix(t,e,n),this.matrixWorld.decompose(Zs,Js,yn),yn.x===1&&yn.y===1&&yn.z===1)this.matrixWorldInverse.copy(this.matrixWorld).invert();else this.matrixWorldInverse.compose(Zs,Js,yn.set(1,1,1)).invert()}clone(){return new this.constructor().copy(this)}}var Vn=new W,vl=new fe,yl=new fe;class $e extends Mr{constructor(t=50,e=1,n=0.1,i=2000){super();this.isPerspectiveCamera=!0,this.type="PerspectiveCamera",this.fov=t,this.zoom=1,this.near=n,this.far=i,this.focus=10,this.aspect=e,this.view=null,this.filmGauge=35,this.filmOffset=0,this.updateProjectionMatrix()}copy(t,e){return super.copy(t,e),this.fov=t.fov,this.zoom=t.zoom,this.near=t.near,this.far=t.far,this.focus=t.focus,this.aspect=t.aspect,this.view=t.view===null?null:Object.assign({},t.view),this.filmGauge=t.filmGauge,this.filmOffset=t.filmOffset,this}setFocalLength(t){let e=0.5*this.getFilmHeight()/t;this.fov=$s*2*Math.atan(e),this.updateProjectionMatrix()}getFocalLength(){let t=Math.tan(Fr*0.5*this.fov);return 0.5*this.getFilmHeight()/t}getEffectiveFOV(){return $s*2*Math.atan(Math.tan(Fr*0.5*this.fov)/this.zoom)}getFilmWidth(){return this.filmGauge*Math.min(this.aspect,1)}getFilmHeight(){return this.filmGauge/Math.max(this.aspect,1)}getViewBounds(t,e,n){Vn.set(-1,-1,0.5).applyMatrix4(this.projectionMatrixInverse),e.set(Vn.x,Vn.y).multiplyScalar(-t/Vn.z),Vn.set(1,1,0.5).applyMatrix4(this.projectionMatrixInverse),n.set(Vn.x,Vn.y).multiplyScalar(-t/Vn.z)}getViewSize(t,e){return this.getViewBounds(t,vl,yl),e.subVectors(yl,vl)}setViewOffset(t,e,n,i,s,r){if(this.aspect=t/e,this.view===null)this.view={enabled:!0,fullWidth:1,fullHeight:1,offsetX:0,offsetY:0,width:1,height:1};this.view.enabled=!0,this.view.fullWidth=t,this.view.fullHeight=e,this.view.offsetX=n,this.view.offsetY=i,this.view.width=s,this.view.height=r,this.updateProjectionMatrix()}clearViewOffset(){if(this.view!==null)this.view.enabled=!1;this.updateProjectionMatrix()}updateProjectionMatrix(){let t=this.near,e=t*Math.tan(Fr*0.5*this.fov)/this.zoom,n=2*e,i=this.aspect*n,s=-0.5*i,r=this.view;if(this.view!==null&&this.view.enabled){let{fullWidth:o,fullHeight:l}=r;s+=r.offsetX*i/o,e-=r.offsetY*n/l,i*=r.width/o,n*=r.height/l}let a=this.filmOffset;if(a!==0)s+=t*a/this.getFilmWidth();this.projectionMatrix.makePerspective(s,s+i,e,e-n,t,this.far,this.coordinateSystem,this.reversedDepth),this.projectionMatrixInverse.copy(this.projectionMatrix).invert()}toJSON(t){let e=super.toJSON(t);if(e.object.fov=this.fov,e.object.zoom=this.zoom,e.object.near=this.near,e.object.far=this.far,e.object.focus=this.focus,e.object.aspect=this.aspect,this.view!==null)e.object.view=Object.assign({},this.view);return e.object.filmGauge=this.filmGauge,e.object.filmOffset=this.filmOffset,e}}class Po extends Mr{constructor(t=-1,e=1,n=1,i=-1,s=0.1,r=2000){super();this.isOrthographicCamera=!0,this.type="OrthographicCamera",this.zoom=1,this.view=null,this.left=t,this.right=e,this.top=n,this.bottom=i,this.near=s,this.far=r,this.updateProjectionMatrix()}copy(t,e){return super.copy(t,e),this.left=t.left,this.right=t.right,this.top=t.top,this.bottom=t.bottom,this.near=t.near,this.far=t.far,this.zoom=t.zoom,this.view=t.view===null?null:Object.assign({},t.view),this}setViewOffset(t,e,n,i,s,r){if(this.view===null)this.view={enabled:!0,fullWidth:1,fullHeight:1,offsetX:0,offsetY:0,width:1,height:1};this.view.enabled=!0,this.view.fullWidth=t,this.view.fullHeight=e,this.view.offsetX=n,this.view.offsetY=i,this.view.width=s,this.view.height=r,this.updateProjectionMatrix()}clearViewOffset(){if(this.view!==null)this.view.enabled=!1;this.updateProjectionMatrix()}updateProjectionMatrix(){let t=(this.right-this.left)/(2*this.zoom),e=(this.top-this.bottom)/(2*this.zoom),n=(this.right+this.left)/2,i=(this.top+this.bottom)/2,s=n-t,r=n+t,a=i+e,o=i-e;if(this.view!==null&&this.view.enabled){let l=(this.right-this.left)/this.view.fullWidth/this.zoom,c=(this.top-this.bottom)/this.view.fullHeight/this.zoom;s+=l*this.view.offsetX,r=s+l*this.view.width,a-=c*this.view.offsetY,o=a-c*this.view.height}this.projectionMatrix.makeOrthographic(s,r,a,o,this.near,this.far,this.coordinateSystem,this.reversedDepth),this.projectionMatrixInverse.copy(this.projectionMatrix).invert()}toJSON(t){let e=super.toJSON(t);if(e.object.zoom=this.zoom,e.object.left=this.left,e.object.right=this.right,e.object.top=this.top,e.object.bottom=this.bottom,e.object.near=this.near,e.object.far=this.far,this.view!==null)e.object.view=Object.assign({},this.view);return e}}class fs extends Ee{constructor(){super();this.isInstancedBufferGeometry=!0,this.type="InstancedBufferGeometry",this.instanceCount=1/0}copy(t){return super.copy(t),this.instanceCount=t.instanceCount,this}toJSON(){let t=super.toJSON();return t.instanceCount=this.instanceCount,t.isInstancedBufferGeometry=!0,t}}var Pi=-90,Li=1;class ps extends Xe{constructor(t,e,n){super();this.type="CubeCamera",this.renderTarget=n,this.coordinateSystem=null,this.activeMipmapLevel=0;let i=new $e(Pi,Li,t,e);i.layers=this.layers,this.add(i);let s=new $e(Pi,Li,t,e);s.layers=this.layers,this.add(s);let r=new $e(Pi,Li,t,e);r.layers=this.layers,this.add(r);let a=new $e(Pi,Li,t,e);a.layers=this.layers,this.add(a);let o=new $e(Pi,Li,t,e);o.layers=this.layers,this.add(o);let l=new $e(Pi,Li,t,e);l.layers=this.layers,this.add(l)}updateCoordinateSystem(){let t=this.coordinateSystem,e=this.children.concat(),[n,i,s,r,a,o]=e;for(let l of e)this.remove(l);if(t===2000)n.up.set(0,1,0),n.lookAt(1,0,0),i.up.set(0,1,0),i.lookAt(-1,0,0),s.up.set(0,0,-1),s.lookAt(0,1,0),r.up.set(0,0,1),r.lookAt(0,-1,0),a.up.set(0,1,0),a.lookAt(0,0,1),o.up.set(0,1,0),o.lookAt(0,0,-1);else if(t===2001)n.up.set(0,-1,0),n.lookAt(-1,0,0),i.up.set(0,-1,0),i.lookAt(1,0,0),s.up.set(0,0,1),s.lookAt(0,1,0),r.up.set(0,0,-1),r.lookAt(0,-1,0),a.up.set(0,-1,0),a.lookAt(0,0,1),o.up.set(0,-1,0),o.lookAt(0,0,-1);else throw Error("THREE.CubeCamera.updateCoordinateSystem(): Invalid coordinate system: "+t);for(let l of e)this.add(l),l.updateMatrixWorld()}update(t,e){if(this.parent===null)this.updateMatrixWorld();let{renderTarget:n,activeMipmapLevel:i}=this;if(this.coordinateSystem!==t.coordinateSystem)this.coordinateSystem=t.coordinateSystem,this.updateCoordinateSystem();let[s,r,a,o,l,c]=this.children,p=t.getRenderTarget(),h=t.getActiveCubeFace(),u=t.getActiveMipmapLevel(),d=t.xr.enabled;t.xr.enabled=!1;let v=n.texture.mipmapsAutoUpdate;n.texture.mipmapsAutoUpdate=!1;let w=!1;if(t.isWebGLRenderer===!0)w=t.state.buffers.depth.getReversed();else w=t.reversedDepthBuffer;if(t.setRenderTarget(n,0,i),w&&t.autoClear===!1)t.clearDepth();if(t.render(e,s),t.setRenderTarget(n,1,i),w&&t.autoClear===!1)t.clearDepth();if(t.render(e,r),t.setRenderTarget(n,2,i),w&&t.autoClear===!1)t.clearDepth();if(t.render(e,a),t.setRenderTarget(n,3,i),w&&t.autoClear===!1)t.clearDepth();if(t.render(e,o),t.setRenderTarget(n,4,i),w&&t.autoClear===!1)t.clearDepth();if(t.render(e,l),n.texture.mipmapsAutoUpdate=v,t.setRenderTarget(n,5,i),w&&t.autoClear===!1)t.clearDepth();t.render(e,c),t.setRenderTarget(p,h,u),t.xr.enabled=d,n.texture.needsPMREMUpdate=!0}}class Lo extends $e{constructor(t=[]){super();this.isArrayCamera=!0,this.isMultiViewCamera=!1,this.cameras=t,this.perCameraFrustumCulled=!1}}var No="\\[\\]\\.:\\/",jh=new RegExp("["+No+"]","g"),Do="[^"+No+"]",tu="[^"+No.replace("\\.","")+"]",eu=/((?:WC+[\/:])*)/.source.replace("WC",Do),nu=/(WCOD+)?/.source.replace("WCOD",tu),iu=/(?:\.(WC+)(?:\[(.+)\])?)?/.source.replace("WC",Do),su=/\.(WC+)(?:\[(.+)\])?/.source.replace("WC",Do),ru=new RegExp("^"+eu+nu+iu+su+"$"),au=["material","materials","bones","map"];class Ec{constructor(t,e,n){let i=n||me.parseTrackName(e);this._targetGroup=t,this._bindings=t.subscribe_(e,i)}getValue(t,e){this.bind();let n=this._targetGroup.nCachedObjects_,i=this._bindings[n];if(i!==void 0)i.getValue(t,e)}setValue(t,e){let n=this._bindings;for(let i=this._targetGroup.nCachedObjects_,s=n.length;i!==s;++i)n[i].setValue(t,e)}bind(){let t=this._bindings;for(let e=this._targetGroup.nCachedObjects_,n=t.length;e!==n;++e)t[e].bind()}unbind(){let t=this._bindings;for(let e=this._targetGroup.nCachedObjects_,n=t.length;e!==n;++e)t[e].unbind()}}class me{constructor(t,e,n){this.path=e,this.parsedPath=n||me.parseTrackName(e),this.node=me.findNode(t,this.parsedPath.nodeName),this.rootNode=t,this.getValue=this._getValue_unbound,this.setValue=this._setValue_unbound}static create(t,e,n){if(!(t&&t.isAnimationObjectGroup))return new me(t,e,n);else return new me.Composite(t,e,n)}static sanitizeNodeName(t){return t.replace(/\s/g,"_").replace(jh,"")}static parseTrackName(t){let e=ru.exec(t);if(e===null)throw Error("THREE.PropertyBinding: Cannot parse trackName: "+t);let n={nodeName:e[2],objectName:e[3],objectIndex:e[4],propertyName:e[5],propertyIndex:e[6]},i=n.nodeName&&n.nodeName.lastIndexOf(".");if(i!==void 0&&i!==-1){let s=n.nodeName.substring(i+1);if(au.indexOf(s)!==-1)n.nodeName=n.nodeName.substring(0,i),n.objectName=s}if(n.propertyName===null||n.propertyName.length===0)throw Error("THREE.PropertyBinding: can not parse propertyName from trackName: "+t);return n}static findNode(t,e){if(e===void 0||e===""||e==="."||e===-1||e===t.name||e===t.uuid)return t;if(t.skeleton){let n=t.skeleton.getBoneByName(e);if(n!==void 0)return n}if(t.children){let n=function(s){for(let r=0;r<s.length;r++){let a=s[r];if(a.name===e||a.uuid===e)return a;let o=n(a.children);if(o)return o}return null},i=n(t.children);if(i)return i}return null}_getValue_unavailable(){}_setValue_unavailable(){}_getValue_direct(t,e){t[e]=this.targetObject[this.propertyName]}_getValue_array(t,e){let n=this.resolvedProperty;for(let i=0,s=n.length;i!==s;++i)t[e++]=n[i]}_getValue_arrayElement(t,e){t[e]=this.resolvedProperty[this.propertyIndex]}_getValue_toArray(t,e){this.resolvedProperty.toArray(t,e)}_setValue_direct(t,e){this.targetObject[this.propertyName]=t[e]}_setValue_direct_setNeedsUpdate(t,e){this.targetObject[this.propertyName]=t[e],this.targetObject.needsUpdate=!0}_setValue_direct_setMatrixWorldNeedsUpdate(t,e){this.targetObject[this.propertyName]=t[e],this.targetObject.matrixWorldNeedsUpdate=!0}_setValue_array(t,e){let n=this.resolvedProperty;for(let i=0,s=n.length;i!==s;++i)n[i]=t[e++]}_setValue_array_setNeedsUpdate(t,e){let n=this.resolvedProperty;for(let i=0,s=n.length;i!==s;++i)n[i]=t[e++];this.targetObject.needsUpdate=!0}_setValue_array_setMatrixWorldNeedsUpdate(t,e){let n=this.resolvedProperty;for(let i=0,s=n.length;i!==s;++i)n[i]=t[e++];this.targetObject.matrixWorldNeedsUpdate=!0}_setValue_arrayElement(t,e){this.resolvedProperty[this.propertyIndex]=t[e]}_setValue_arrayElement_setNeedsUpdate(t,e){this.resolvedProperty[this.propertyIndex]=t[e],this.targetObject.needsUpdate=!0}_setValue_arrayElement_setMatrixWorldNeedsUpdate(t,e){this.resolvedProperty[this.propertyIndex]=t[e],this.targetObject.matrixWorldNeedsUpdate=!0}_setValue_fromArray(t,e){this.resolvedProperty.fromArray(t,e)}_setValue_fromArray_setNeedsUpdate(t,e){this.resolvedProperty.fromArray(t,e),this.targetObject.needsUpdate=!0}_setValue_fromArray_setMatrixWorldNeedsUpdate(t,e){this.resolvedProperty.fromArray(t,e),this.targetObject.matrixWorldNeedsUpdate=!0}_getValue_unbound(t,e){this.bind(),this.getValue(t,e)}_setValue_unbound(t,e){this.bind(),this.setValue(t,e)}bind(){let t=this.node,e=this.parsedPath,{objectName:n,propertyName:i,propertyIndex:s}=e;if(!t)t=me.findNode(this.rootNode,e.nodeName),this.node=t;if(this.getValue=this._getValue_unavailable,this.setValue=this._setValue_unavailable,!t){Wt("PropertyBinding: No target node found for track: "+this.path+".");return}if(n){let l=e.objectIndex;switch(n){case"materials":if(!t.material){Yt("PropertyBinding: Can not bind to material as node does not have a material.",this);return}if(!t.material.materials){Yt("PropertyBinding: Can not bind to material.materials as node.material does not have a materials array.",this);return}t=t.material.materials;break;case"bones":if(!t.skeleton){Yt("PropertyBinding: Can not bind to bones as node does not have a skeleton.",this);return}t=t.skeleton.bones;for(let c=0;c<t.length;c++)if(t[c].name===l){l=c;break}break;case"map":if("map"in t){t=t.map;break}if(!t.material){Yt("PropertyBinding: Can not bind to material as node does not have a material.",this);return}if(!t.material.map){Yt("PropertyBinding: Can not bind to material.map as node.material does not have a map.",this);return}t=t.material.map;break;default:if(t[n]===void 0){Yt("PropertyBinding: Can not bind to objectName of node undefined.",this);return}t=t[n]}if(l!==void 0){if(t[l]===void 0){Yt("PropertyBinding: Trying to bind to objectIndex of objectName, but is undefined.",this,t);return}t=t[l]}}let r=t[i];if(r===void 0){let l=e.nodeName;Yt("PropertyBinding: Trying to update property for track: "+l+"."+i+" but it wasn't found.",t);return}let a=this.Versioning.None;if(this.targetObject=t,t.isMaterial===!0)a=this.Versioning.NeedsUpdate;else if(t.isObject3D===!0)a=this.Versioning.MatrixWorldNeedsUpdate;let o=this.BindingType.Direct;if(s!==void 0){if(i==="morphTargetInfluences"){if(!t.geometry){Yt("PropertyBinding: Can not bind to morphTargetInfluences because node does not have a geometry.",this);return}if(!t.geometry.morphAttributes){Yt("PropertyBinding: Can not bind to morphTargetInfluences because node does not have a geometry.morphAttributes.",this);return}if(t.morphTargetDictionary[s]!==void 0)s=t.morphTargetDictionary[s]}o=this.BindingType.ArrayElement,this.resolvedProperty=r,this.propertyIndex=s}else if(r.fromArray!==void 0&&r.toArray!==void 0)o=this.BindingType.HasFromToArray,this.resolvedProperty=r;else if(Array.isArray(r))o=this.BindingType.EntireArray,this.resolvedProperty=r;else this.propertyName=i;this.getValue=this.GetterByBindingType[o],this.setValue=this.SetterByBindingTypeAndVersioning[o][a]}unbind(){this.node=null,this.getValue=this._getValue_unbound,this.setValue=this._setValue_unbound}}me.Composite=Ec;me.prototype.BindingType={Direct:0,EntireArray:1,ArrayElement:2,HasFromToArray:3};me.prototype.Versioning={None:0,NeedsUpdate:1,MatrixWorldNeedsUpdate:2};me.prototype.GetterByBindingType=[me.prototype._getValue_direct,me.prototype._getValue_array,me.prototype._getValue_arrayElement,me.prototype._getValue_toArray];me.prototype.SetterByBindingTypeAndVersioning=[[me.prototype._setValue_direct,me.prototype._setValue_direct_setNeedsUpdate,me.prototype._setValue_direct_setMatrixWorldNeedsUpdate],[me.prototype._setValue_array,me.prototype._setValue_array_setNeedsUpdate,me.prototype._setValue_array_setMatrixWorldNeedsUpdate],[me.prototype._setValue_arrayElement,me.prototype._setValue_arrayElement_setNeedsUpdate,me.prototype._setValue_arrayElement_setMatrixWorldNeedsUpdate],[me.prototype._setValue_fromArray,me.prototype._setValue_fromArray_setNeedsUpdate,me.prototype._setValue_fromArray_setMatrixWorldNeedsUpdate]];var wg=new Float32Array(1);class Rc{static{this.prototype.isMatrix2=!0}constructor(t,e,n,i){if(this.elements=[1,0,0,1],t!==void 0)this.set(t,e,n,i)}identity(){return this.set(1,0,0,1),this}fromArray(t,e=0){for(let n=0;n<4;n++)this.elements[n]=t[n+e];return this}set(t,e,n,i){let s=this.elements;return s[0]=t,s[2]=e,s[1]=n,s[3]=i,this}}function Uo(t,e,n,i){let s=ou(i);switch(n){case 1021:return t*e;case 1028:return t*e/s.components*s.byteLength;case 1029:return t*e/s.components*s.byteLength;case 1030:return t*e*2/s.components*s.byteLength;case 1031:return t*e*2/s.components*s.byteLength;case 1022:return t*e*3/s.components*s.byteLength;case 1023:return t*e*4/s.components*s.byteLength;case 1033:return t*e*4/s.components*s.byteLength;case 33776:case 33777:return Math.floor((t+3)/4)*Math.floor((e+3)/4)*8;case 33778:case 33779:return Math.floor((t+3)/4)*Math.floor((e+3)/4)*16;case 35841:case 35843:return Math.max(t,16)*Math.max(e,8)/4;case 35840:case 35842:return Math.max(t,8)*Math.max(e,8)/2;case 36196:case 37492:case 37488:case 37489:return Math.floor((t+3)/4)*Math.floor((e+3)/4)*8;case 37496:case 37490:case 37491:return Math.floor((t+3)/4)*Math.floor((e+3)/4)*16;case 37808:return Math.floor((t+3)/4)*Math.floor((e+3)/4)*16;case 37809:return Math.floor((t+4)/5)*Math.floor((e+3)/4)*16;case 37810:return Math.floor((t+4)/5)*Math.floor((e+4)/5)*16;case 37811:return Math.floor((t+5)/6)*Math.floor((e+4)/5)*16;case 37812:return Math.floor((t+5)/6)*Math.floor((e+5)/6)*16;case 37813:return Math.floor((t+7)/8)*Math.floor((e+4)/5)*16;case 37814:return Math.floor((t+7)/8)*Math.floor((e+5)/6)*16;case 37815:return Math.floor((t+7)/8)*Math.floor((e+7)/8)*16;case 37816:return Math.floor((t+9)/10)*Math.floor((e+4)/5)*16;case 37817:return Math.floor((t+9)/10)*Math.floor((e+5)/6)*16;case 37818:return Math.floor((t+9)/10)*Math.floor((e+7)/8)*16;case 37819:return Math.floor((t+9)/10)*Math.floor((e+9)/10)*16;case 37820:return Math.floor((t+11)/12)*Math.floor((e+9)/10)*16;case 37821:return Math.floor((t+11)/12)*Math.floor((e+11)/12)*16;case 36492:case 36494:case 36495:return Math.ceil(t/4)*Math.ceil(e/4)*16;case 36283:case 36284:return Math.ceil(t/4)*Math.ceil(e/4)*8;case 36285:case 36286:return Math.ceil(t/4)*Math.ceil(e/4)*16}throw Error(`Unable to determine texture byte length for ${n} format.`)}function ou(t){switch(t){case 1009:case 1010:return{byteLength:1,components:1};case 1012:case 1011:case 1016:return{byteLength:2,components:1};case 1017:case 1018:return{byteLength:2,components:4};case 1014:case 1013:case 1015:return{byteLength:4,components:1};case 35902:case 35899:return{byteLength:4,components:3}}throw Error(`THREE.TextureUtils: Unknown texture type ${t}.`)}if(typeof __THREE_DEVTOOLS__<"u")__THREE_DEVTOOLS__.dispatchEvent(new CustomEvent("register",{detail:{revision:"187"}}));if(typeof window<"u")if(window.__THREE__)Wt("WARNING: Multiple instances of Three.js being imported.");else window.__THREE__="187";function $c(){let t=null,e=!1,n=null,i=null;function s(r,a){i=t.requestAnimationFrame(s),n(r,a)}return{start:function(){if(e===!0)return;if(n===null)return;if(t===null)return;i=t.requestAnimationFrame(s),e=!0},stop:function(){if(t!==null)t.cancelAnimationFrame(i);e=!1},setAnimationLoop:function(r){n=r},setContext:function(r){t=r}}}function lu(t){let e=new WeakMap;function n(o,l){let{array:c,usage:p}=o,h=c.byteLength,u=t.createBuffer();t.bindBuffer(l,u),t.bufferData(l,c,p),o.onUploadCallback();let d;if(c instanceof Float32Array)d=t.FLOAT;else if(typeof Float16Array<"u"&&c instanceof Float16Array)d=t.HALF_FLOAT;else if(c instanceof Uint16Array)if(o.isFloat16BufferAttribute)d=t.HALF_FLOAT;else d=t.UNSIGNED_SHORT;else if(c instanceof Int16Array)d=t.SHORT;else if(c instanceof Uint32Array)d=t.UNSIGNED_INT;else if(c instanceof Int32Array)d=t.INT;else if(c instanceof Int8Array)d=t.BYTE;else if(c instanceof Uint8Array)d=t.UNSIGNED_BYTE;else if(c instanceof Uint8ClampedArray)d=t.UNSIGNED_BYTE;else throw Error("THREE.WebGLAttributes: Unsupported buffer data format: "+c);return{buffer:u,type:d,bytesPerElement:c.BYTES_PER_ELEMENT,version:o.version,size:h}}function i(o,l,c){let{array:p,updateRanges:h}=l;if(t.bindBuffer(c,o),h.length===0)t.bufferSubData(c,0,p);else{h.sort((d,v)=>d.start-v.start);let u=0;for(let d=1;d<h.length;d++){let v=h[u],w=h[d];if(w.start<=v.start+v.count+1)v.count=Math.max(v.count,w.start+w.count-v.start);else++u,h[u]=w}h.length=u+1;for(let d=0,v=h.length;d<v;d++){let w=h[d];t.bufferSubData(c,w.start*p.BYTES_PER_ELEMENT,p,w.start,w.count)}l.clearUpdateRanges()}l.onUploadCallback()}function s(o){if(o.isInterleavedBufferAttribute)o=o.data;return e.get(o)}function r(o){if(o.isInterleavedBufferAttribute)o=o.data;let l=e.get(o);if(l)t.deleteBuffer(l.buffer),e.delete(o)}function a(o,l){if(o.isInterleavedBufferAttribute)o=o.data;if(o.isGLBufferAttribute){let p=e.get(o);if(!p||p.version<o.version)e.set(o,{buffer:o.buffer,type:o.type,bytesPerElement:o.elementSize,version:o.version});return}let c=e.get(o);if(c===void 0)e.set(o,n(o,l));else if(c.version<o.version){if(c.size!==o.array.byteLength)throw Error("THREE.WebGLAttributes: The size of the buffer attribute's array buffer does not match the original size. Resizing buffer attributes is not supported.");i(c.buffer,o,l),c.version=o.version}}return{get:s,remove:r,update:a}}var cu=`#ifdef USE_ALPHAHASH
	if ( diffuseColor.a < getAlphaHashThreshold( vPosition ) ) discard;
#endif`,hu=`#ifdef USE_ALPHAHASH
	const float ALPHA_HASH_SCALE = 0.05;
	float hash2D( vec2 value ) {
		return fract( 1.0e4 * sin( 17.0 * value.x + 0.1 * value.y ) * ( 0.1 + abs( sin( 13.0 * value.y + value.x ) ) ) );
	}
	float hash3D( vec3 value ) {
		return hash2D( vec2( hash2D( value.xy ), value.z ) );
	}
	float getAlphaHashThreshold( vec3 position ) {
		float maxDeriv = max(
			length( dFdx( position.xyz ) ),
			length( dFdy( position.xyz ) )
		);
		float pixScale = 1.0 / ( ALPHA_HASH_SCALE * maxDeriv );
		vec2 pixScales = vec2(
			exp2( floor( log2( pixScale ) ) ),
			exp2( ceil( log2( pixScale ) ) )
		);
		vec2 alpha = vec2(
			hash3D( floor( pixScales.x * position.xyz ) ),
			hash3D( floor( pixScales.y * position.xyz ) )
		);
		float lerpFactor = fract( log2( pixScale ) );
		float x = ( 1.0 - lerpFactor ) * alpha.x + lerpFactor * alpha.y;
		float a = min( lerpFactor, 1.0 - lerpFactor );
		vec3 cases = vec3(
			x * x / ( 2.0 * a * ( 1.0 - a ) ),
			( x - 0.5 * a ) / ( 1.0 - a ),
			1.0 - ( ( 1.0 - x ) * ( 1.0 - x ) / ( 2.0 * a * ( 1.0 - a ) ) )
		);
		float threshold = ( x < ( 1.0 - a ) )
			? ( ( x < a ) ? cases.x : cases.y )
			: cases.z;
		return clamp( threshold , 1.0e-6, 1.0 );
	}
#endif`,uu=`#ifdef USE_ALPHAMAP
	diffuseColor.a *= texture2D( alphaMap, vAlphaMapUv ).g;
#endif`,du=`#ifdef USE_ALPHAMAP
	uniform sampler2D alphaMap;
#endif`,fu=`#ifdef USE_ALPHATEST
	#ifdef ALPHA_TO_COVERAGE
	diffuseColor.a = smoothstep( alphaTest, alphaTest + fwidth( diffuseColor.a ), diffuseColor.a );
	if ( diffuseColor.a == 0.0 ) discard;
	#else
	if ( diffuseColor.a < alphaTest ) discard;
	#endif
#endif`,pu=`#ifdef USE_ALPHATEST
	uniform float alphaTest;
#endif`,mu=`#ifdef USE_AOMAP
	float ambientOcclusion = ( texture2D( aoMap, vAoMapUv ).r - 1.0 ) * aoMapIntensity + 1.0;
	reflectedLight.indirectDiffuse *= ambientOcclusion;
	#if defined( USE_CLEARCOAT ) 
		clearcoatSpecularIndirect *= ambientOcclusion;
	#endif
	#if defined( USE_SHEEN ) 
		sheenSpecularIndirect *= ambientOcclusion;
	#endif
	#if defined( USE_ENVMAP ) && defined( STANDARD )
		float dotNV = saturate( dot( geometryNormal, geometryViewDir ) );
		reflectedLight.indirectSpecular *= computeSpecularOcclusion( dotNV, ambientOcclusion, material.roughness );
	#endif
#endif`,gu=`#ifdef USE_AOMAP
	uniform sampler2D aoMap;
	uniform float aoMapIntensity;
#endif`,_u=`#ifdef USE_BATCHING
	#if ! defined( GL_ANGLE_multi_draw )
	#define gl_DrawID _gl_DrawID
	uniform int _gl_DrawID;
	#endif
	uniform highp sampler2D batchingTexture;
	uniform highp usampler2D batchingIdTexture;
	mat4 getBatchingMatrix( const in float i ) {
		int size = textureSize( batchingTexture, 0 ).x;
		int j = int( i ) * 4;
		int x = j % size;
		int y = j / size;
		vec4 v1 = texelFetch( batchingTexture, ivec2( x, y ), 0 );
		vec4 v2 = texelFetch( batchingTexture, ivec2( x + 1, y ), 0 );
		vec4 v3 = texelFetch( batchingTexture, ivec2( x + 2, y ), 0 );
		vec4 v4 = texelFetch( batchingTexture, ivec2( x + 3, y ), 0 );
		return mat4( v1, v2, v3, v4 );
	}
	float getIndirectIndex( const in int i ) {
		int size = textureSize( batchingIdTexture, 0 ).x;
		int x = i % size;
		int y = i / size;
		return float( texelFetch( batchingIdTexture, ivec2( x, y ), 0 ).r );
	}
#endif
#ifdef USE_BATCHING_COLOR
	uniform sampler2D batchingColorTexture;
	vec4 getBatchingColor( const in float i ) {
		int size = textureSize( batchingColorTexture, 0 ).x;
		int j = int( i );
		int x = j % size;
		int y = j / size;
		return texelFetch( batchingColorTexture, ivec2( x, y ), 0 );
	}
#endif`,xu=`#ifdef USE_BATCHING
	mat4 batchingMatrix = getBatchingMatrix( getIndirectIndex( gl_DrawID ) );
#endif`,vu=`vec3 transformed = vec3( position );
#ifdef USE_ALPHAHASH
	vPosition = vec3( position );
#endif`,yu=`vec3 objectNormal = vec3( normal );
#ifdef USE_TANGENT
	vec3 objectTangent = vec3( tangent.xyz );
#endif`,Mu=`float G_BlinnPhong_Implicit( ) {
	return 0.25;
}
float D_BlinnPhong( const in float shininess, const in float dotNH ) {
	return RECIPROCAL_PI * ( shininess * 0.5 + 1.0 ) * pow( dotNH, shininess );
}
vec3 BRDF_BlinnPhong( const in vec3 lightDir, const in vec3 viewDir, const in vec3 normal, const in vec3 specularColor, const in float shininess ) {
	vec3 halfDir = normalize( lightDir + viewDir );
	float dotNH = saturate( dot( normal, halfDir ) );
	float dotVH = saturate( dot( viewDir, halfDir ) );
	vec3 F = F_Schlick( specularColor, 1.0, dotVH );
	float G = G_BlinnPhong_Implicit( );
	float D = D_BlinnPhong( shininess, dotNH );
	return F * ( G * D );
} // validated`,Su=`#ifdef USE_IRIDESCENCE
	const mat3 XYZ_TO_REC709 = mat3(
		 3.2404542, -0.9692660,  0.0556434,
		-1.5371385,  1.8760108, -0.2040259,
		-0.4985314,  0.0415560,  1.0572252
	);
	vec3 Fresnel0ToIor( vec3 fresnel0 ) {
		vec3 sqrtF0 = sqrt( fresnel0 );
		return ( vec3( 1.0 ) + sqrtF0 ) / ( vec3( 1.0 ) - sqrtF0 );
	}
	vec3 IorToFresnel0( vec3 transmittedIor, float incidentIor ) {
		return pow2( ( transmittedIor - vec3( incidentIor ) ) / ( transmittedIor + vec3( incidentIor ) ) );
	}
	float IorToFresnel0( float transmittedIor, float incidentIor ) {
		return pow2( ( transmittedIor - incidentIor ) / ( transmittedIor + incidentIor ));
	}
	vec3 evalSensitivity( float OPD, vec3 shift ) {
		float phase = 2.0 * PI * OPD * 1.0e-9;
		vec3 val = vec3( 5.4856e-13, 4.4201e-13, 5.2481e-13 );
		vec3 pos = vec3( 1.6810e+06, 1.7953e+06, 2.2084e+06 );
		vec3 var = vec3( 4.3278e+09, 9.3046e+09, 6.6121e+09 );
		vec3 xyz = val * sqrt( 2.0 * PI * var ) * cos( pos * phase + shift ) * exp( - pow2( phase ) * var );
		xyz.x += 9.7470e-14 * sqrt( 2.0 * PI * 4.5282e+09 ) * cos( 2.2399e+06 * phase + shift[ 0 ] ) * exp( - 4.5282e+09 * pow2( phase ) );
		xyz /= 1.0685e-7;
		vec3 rgb = XYZ_TO_REC709 * xyz;
		return rgb;
	}
	vec3 evalIridescence( float outsideIOR, float eta2, float cosTheta1, float thinFilmThickness, vec3 baseF0 ) {
		vec3 I;
		float iridescenceIOR = mix( outsideIOR, eta2, smoothstep( 0.0, 0.03, thinFilmThickness ) );
		float sinTheta2Sq = pow2( outsideIOR / iridescenceIOR ) * ( 1.0 - pow2( cosTheta1 ) );
		float cosTheta2Sq = 1.0 - sinTheta2Sq;
		if ( cosTheta2Sq < 0.0 ) {
			return vec3( 1.0 );
		}
		float cosTheta2 = sqrt( cosTheta2Sq );
		float R0 = IorToFresnel0( iridescenceIOR, outsideIOR );
		float R12 = F_Schlick( R0, 1.0, cosTheta1 );
		float T121 = 1.0 - R12;
		float phi12 = 0.0;
		if ( iridescenceIOR < outsideIOR ) phi12 = PI;
		float phi21 = PI - phi12;
		vec3 baseIOR = Fresnel0ToIor( clamp( baseF0, 0.0, 0.9999 ) );		vec3 R1 = IorToFresnel0( baseIOR, iridescenceIOR );
		vec3 R23 = F_Schlick( R1, 1.0, cosTheta2 );
		vec3 phi23 = vec3( 0.0 );
		if ( baseIOR[ 0 ] < iridescenceIOR ) phi23[ 0 ] = PI;
		if ( baseIOR[ 1 ] < iridescenceIOR ) phi23[ 1 ] = PI;
		if ( baseIOR[ 2 ] < iridescenceIOR ) phi23[ 2 ] = PI;
		float OPD = 2.0 * iridescenceIOR * thinFilmThickness * cosTheta2;
		vec3 phi = vec3( phi21 ) + phi23;
		vec3 R123 = clamp( R12 * R23, 1e-5, 0.9999 );
		vec3 r123 = sqrt( R123 );
		vec3 Rs = pow2( T121 ) * R23 / ( vec3( 1.0 ) - R123 );
		vec3 C0 = R12 + Rs;
		I = C0;
		vec3 Cm = Rs - T121;
		for ( int m = 1; m <= 2; ++ m ) {
			Cm *= r123;
			vec3 Sm = 2.0 * evalSensitivity( float( m ) * OPD, float( m ) * phi );
			I += Cm * Sm;
		}
		return max( I, vec3( 0.0 ) );
	}
#endif`,bu=`#ifdef USE_BUMPMAP
	uniform sampler2D bumpMap;
	uniform float bumpScale;
	vec2 dHdxy_fwd() {
		vec2 dSTdx = dFdx( vBumpMapUv );
		vec2 dSTdy = dFdy( vBumpMapUv );
		float Hll = bumpScale * texture2D( bumpMap, vBumpMapUv ).x;
		float dBx = bumpScale * texture2D( bumpMap, vBumpMapUv + dSTdx ).x - Hll;
		float dBy = bumpScale * texture2D( bumpMap, vBumpMapUv + dSTdy ).x - Hll;
		return vec2( dBx, dBy );
	}
	vec3 perturbNormalArb( vec3 surf_pos, vec3 surf_norm, vec2 dHdxy, float faceDirection ) {
		vec3 vSigmaX = normalize( dFdx( surf_pos.xyz ) );
		vec3 vSigmaY = normalize( dFdy( surf_pos.xyz ) );
		vec3 vN = surf_norm;
		vec3 R1 = cross( vSigmaY, vN );
		vec3 R2 = cross( vN, vSigmaX );
		float fDet = dot( vSigmaX, R1 ) * faceDirection;
		vec3 vGrad = sign( fDet ) * ( dHdxy.x * R1 + dHdxy.y * R2 );
		return normalize( abs( fDet ) * surf_norm - vGrad );
	}
#endif`,wu=`#if NUM_CLIPPING_PLANES > 0
	vec4 plane;
	#ifdef ALPHA_TO_COVERAGE
		float distanceToPlane, distanceGradient;
		float clipOpacity = 1.0;
		#pragma unroll_loop_start
		for ( int i = 0; i < UNION_CLIPPING_PLANES; i ++ ) {
			plane = clippingPlanes[ i ];
			distanceToPlane = - dot( vClipPosition, plane.xyz ) + plane.w;
			distanceGradient = fwidth( distanceToPlane ) / 2.0;
			clipOpacity *= smoothstep( - distanceGradient, distanceGradient, distanceToPlane );
			if ( clipOpacity == 0.0 ) discard;
		}
		#pragma unroll_loop_end
		#if UNION_CLIPPING_PLANES < NUM_CLIPPING_PLANES
			float unionClipOpacity = 1.0;
			#pragma unroll_loop_start
			for ( int i = UNION_CLIPPING_PLANES; i < NUM_CLIPPING_PLANES; i ++ ) {
				plane = clippingPlanes[ i ];
				distanceToPlane = - dot( vClipPosition, plane.xyz ) + plane.w;
				distanceGradient = fwidth( distanceToPlane ) / 2.0;
				unionClipOpacity *= 1.0 - smoothstep( - distanceGradient, distanceGradient, distanceToPlane );
			}
			#pragma unroll_loop_end
			clipOpacity *= 1.0 - unionClipOpacity;
		#endif
		diffuseColor.a *= clipOpacity;
		if ( diffuseColor.a == 0.0 ) discard;
	#else
		#pragma unroll_loop_start
		for ( int i = 0; i < UNION_CLIPPING_PLANES; i ++ ) {
			plane = clippingPlanes[ i ];
			if ( dot( vClipPosition, plane.xyz ) > plane.w ) discard;
		}
		#pragma unroll_loop_end
		#if UNION_CLIPPING_PLANES < NUM_CLIPPING_PLANES
			bool clipped = true;
			#pragma unroll_loop_start
			for ( int i = UNION_CLIPPING_PLANES; i < NUM_CLIPPING_PLANES; i ++ ) {
				plane = clippingPlanes[ i ];
				clipped = ( dot( vClipPosition, plane.xyz ) > plane.w ) && clipped;
			}
			#pragma unroll_loop_end
			if ( clipped ) discard;
		#endif
	#endif
#endif`,Au=`#if NUM_CLIPPING_PLANES > 0
	varying vec3 vClipPosition;
	uniform vec4 clippingPlanes[ NUM_CLIPPING_PLANES ];
#endif`,Tu=`#if NUM_CLIPPING_PLANES > 0
	varying vec3 vClipPosition;
#endif`,Eu=`#if NUM_CLIPPING_PLANES > 0
	vClipPosition = - mvPosition.xyz;
#endif`,Ru=`#if defined( USE_COLOR ) || defined( USE_COLOR_ALPHA )
	diffuseColor *= vColor;
#endif`,Cu=`#if defined( USE_COLOR ) || defined( USE_COLOR_ALPHA )
	varying vec4 vColor;
#endif`,Iu=`#if defined( USE_COLOR ) || defined( USE_COLOR_ALPHA ) || defined( USE_INSTANCING_COLOR ) || defined( USE_BATCHING_COLOR )
	varying vec4 vColor;
#endif`,Pu=`#if defined( USE_COLOR ) || defined( USE_COLOR_ALPHA ) || defined( USE_INSTANCING_COLOR ) || defined( USE_BATCHING_COLOR )
	vColor = vec4( 1.0 );
#endif
#ifdef USE_COLOR_ALPHA
	vColor *= color;
#elif defined( USE_COLOR )
	vColor.rgb *= color;
#endif
#ifdef USE_INSTANCING_COLOR
	vColor.rgb *= instanceColor.rgb;
#endif
#ifdef USE_BATCHING_COLOR
	vColor *= getBatchingColor( getIndirectIndex( gl_DrawID ) );
#endif`,Lu=`#define PI 3.141592653589793
#define PI2 6.283185307179586
#define PI_HALF 1.5707963267948966
#define RECIPROCAL_PI 0.3183098861837907
#define RECIPROCAL_PI2 0.15915494309189535
#define EPSILON 1e-6
#ifndef saturate
#define saturate( a ) clamp( a, 0.0, 1.0 )
#endif
#define whiteComplement( a ) ( 1.0 - saturate( a ) )
float pow2( const in float x ) { return x*x; }
vec3 pow2( const in vec3 x ) { return x*x; }
float pow3( const in float x ) { return x*x*x; }
float pow4( const in float x ) { float x2 = x*x; return x2*x2; }
float max3( const in vec3 v ) { return max( max( v.x, v.y ), v.z ); }
float average( const in vec3 v ) { return dot( v, vec3( 0.3333333 ) ); }
highp float rand( const in vec2 uv ) {
	const highp float a = 12.9898, b = 78.233, c = 43758.5453;
	highp float dt = dot( uv.xy, vec2( a,b ) ), sn = mod( dt, PI );
	return fract( sin( sn ) * c );
}
#ifdef HIGH_PRECISION
	float precisionSafeLength( vec3 v ) { return length( v ); }
#else
	float precisionSafeLength( vec3 v ) {
		float maxComponent = max3( abs( v ) );
		return length( v / maxComponent ) * maxComponent;
	}
#endif
struct IncidentLight {
	vec3 color;
	vec3 direction;
	bool visible;
};
struct ReflectedLight {
	vec3 directDiffuse;
	vec3 directSpecular;
	vec3 indirectDiffuse;
	vec3 indirectSpecular;
};
#ifdef USE_ALPHAHASH
	varying vec3 vPosition;
#endif
vec3 transformDirection( in vec3 dir, in mat4 matrix ) {
	return normalize( ( matrix * vec4( dir, 0.0 ) ).xyz );
}
#define inverseTransformDirection transformDirectionByInverseViewMatrix
vec3 transformNormalByInverseViewMatrix( in vec3 normal, in mat4 viewMatrix ) {
	return normalize( ( vec4( normal, 0.0 ) * viewMatrix ).xyz );
}
vec3 transformDirectionByInverseViewMatrix( in vec3 dir, in mat4 viewMatrix ) {
	return normalize( ( vec4( dir, 0.0 ) * viewMatrix ).xyz );
}
bool isPerspectiveMatrix( mat4 m ) {
	return m[ 2 ][ 3 ] == - 1.0;
}
vec2 equirectUv( in vec3 dir ) {
	float u = atan( dir.z, dir.x ) * RECIPROCAL_PI2 + 0.5;
	float v = asin( clamp( dir.y, - 1.0, 1.0 ) ) * RECIPROCAL_PI + 0.5;
	return vec2( u, v );
}
vec3 BRDF_Lambert( const in vec3 diffuseColor ) {
	return RECIPROCAL_PI * diffuseColor;
}
vec3 F_Schlick( const in vec3 f0, const in float f90, const in float dotVH ) {
	float fresnel = exp2( ( - 5.55473 * dotVH - 6.98316 ) * dotVH );
	return f0 * ( 1.0 - fresnel ) + ( f90 * fresnel );
}
float F_Schlick( const in float f0, const in float f90, const in float dotVH ) {
	float fresnel = exp2( ( - 5.55473 * dotVH - 6.98316 ) * dotVH );
	return f0 * ( 1.0 - fresnel ) + ( f90 * fresnel );
} // validated`,Nu=`vec3 transformedNormal = objectNormal;
#ifdef USE_TANGENT
	vec3 transformedTangent = objectTangent;
#endif
#ifdef USE_BATCHING
	mat3 bm = mat3( batchingMatrix );
	transformedNormal /= vec3( dot( bm[ 0 ], bm[ 0 ] ), dot( bm[ 1 ], bm[ 1 ] ), dot( bm[ 2 ], bm[ 2 ] ) );
	transformedNormal = bm * transformedNormal;
	#ifdef USE_TANGENT
		transformedTangent = bm * transformedTangent;
	#endif
#endif
#ifdef USE_INSTANCING
	mat3 im = mat3( instanceMatrix );
	transformedNormal /= vec3( dot( im[ 0 ], im[ 0 ] ), dot( im[ 1 ], im[ 1 ] ), dot( im[ 2 ], im[ 2 ] ) );
	transformedNormal = im * transformedNormal;
	#ifdef USE_TANGENT
		transformedTangent = im * transformedTangent;
	#endif
#endif
transformedNormal = normalMatrix * transformedNormal;
#ifdef FLIP_SIDED
	transformedNormal = - transformedNormal;
#endif
#ifdef USE_TANGENT
	transformedTangent = ( modelViewMatrix * vec4( transformedTangent, 0.0 ) ).xyz;
#endif`,Du=`#ifdef USE_DISPLACEMENTMAP
	uniform sampler2D displacementMap;
	uniform float displacementScale;
	uniform float displacementBias;
#endif`,Uu=`#ifdef USE_DISPLACEMENTMAP
	transformed += normalize( objectNormal ) * ( texture2D( displacementMap, vDisplacementMapUv ).x * displacementScale + displacementBias );
#endif`,Fu=`#ifdef USE_EMISSIVEMAP
	vec4 emissiveColor = texture2D( emissiveMap, vEmissiveMapUv );
	#ifdef DECODE_VIDEO_TEXTURE_EMISSIVE
		emissiveColor = sRGBTransferEOTF( emissiveColor );
	#endif
	totalEmissiveRadiance *= emissiveColor.rgb;
#endif`,Ou=`#ifdef USE_EMISSIVEMAP
	uniform sampler2D emissiveMap;
#endif`,Bu="gl_FragColor = linearToOutputTexel( gl_FragColor );",zu=`vec4 LinearTransferOETF( in vec4 value ) {
	return value;
}
vec4 sRGBTransferEOTF( in vec4 value ) {
	return vec4( mix( pow( value.rgb * 0.9478672986 + vec3( 0.0521327014 ), vec3( 2.4 ) ), value.rgb * 0.0773993808, vec3( lessThanEqual( value.rgb, vec3( 0.04045 ) ) ) ), value.a );
}
vec4 sRGBTransferOETF( in vec4 value ) {
	return vec4( mix( pow( value.rgb, vec3( 0.41666 ) ) * 1.055 - vec3( 0.055 ), value.rgb * 12.92, vec3( lessThanEqual( value.rgb, vec3( 0.0031308 ) ) ) ), value.a );
}`,ku=`#ifdef USE_ENVMAP
	#ifdef ENV_WORLDPOS
		vec3 cameraToFrag;
		if ( isOrthographic ) {
			cameraToFrag = normalize( vec3( - viewMatrix[ 0 ][ 2 ], - viewMatrix[ 1 ][ 2 ], - viewMatrix[ 2 ][ 2 ] ) );
		} else {
			cameraToFrag = normalize( vWorldPosition - cameraPosition );
		}
		vec3 worldNormal = transformNormalByInverseViewMatrix( normal, viewMatrix );
		#ifdef ENVMAP_MODE_REFLECTION
			vec3 reflectVec = reflect( cameraToFrag, worldNormal );
		#else
			vec3 reflectVec = refract( cameraToFrag, worldNormal, refractionRatio );
		#endif
	#else
		vec3 reflectVec = vReflect;
	#endif
	#ifdef ENVMAP_TYPE_CUBE
		vec4 envColor = textureCube( envMap, envMapRotation * reflectVec );
		#ifdef ENVMAP_BLENDING_MULTIPLY
			outgoingLight = mix( outgoingLight, outgoingLight * envColor.xyz, specularStrength * reflectivity );
		#elif defined( ENVMAP_BLENDING_MIX )
			outgoingLight = mix( outgoingLight, envColor.xyz, specularStrength * reflectivity );
		#elif defined( ENVMAP_BLENDING_ADD )
			outgoingLight += envColor.xyz * specularStrength * reflectivity;
		#endif
	#endif
#endif`,Gu=`#ifdef USE_ENVMAP
	uniform float envMapIntensity;
	uniform mat3 envMapRotation;
	uniform samplerCube envMap;
	#ifdef ENVMAP_TYPE_PMREM
		float roughnessToMip( const in float roughness ) {
			float r = clamp( roughness, 0.0, 1.0 );
			return ENVMAP_MAX_LOD * r * ( 2.0 - r );
		}
	#endif
#endif`,Hu=`#ifdef USE_ENVMAP
	uniform float reflectivity;
	#if defined( USE_BUMPMAP ) || defined( USE_NORMALMAP ) || defined( PHONG ) || defined( LAMBERT )
		#define ENV_WORLDPOS
	#endif
	#ifdef ENV_WORLDPOS
		varying vec3 vWorldPosition;
		uniform float refractionRatio;
	#else
		varying vec3 vReflect;
	#endif
#endif`,Wu=`#ifdef USE_ENVMAP
	#if defined( USE_BUMPMAP ) || defined( USE_NORMALMAP ) || defined( PHONG ) || defined( LAMBERT )
		#define ENV_WORLDPOS
	#endif
	#ifdef ENV_WORLDPOS
		
		varying vec3 vWorldPosition;
	#else
		varying vec3 vReflect;
		uniform float refractionRatio;
	#endif
#endif`,Vu=`#ifdef USE_ENVMAP
	#ifdef ENV_WORLDPOS
		vWorldPosition = worldPosition.xyz;
	#else
		vec3 cameraToVertex;
		if ( isOrthographic ) {
			cameraToVertex = normalize( vec3( - viewMatrix[ 0 ][ 2 ], - viewMatrix[ 1 ][ 2 ], - viewMatrix[ 2 ][ 2 ] ) );
		} else {
			cameraToVertex = normalize( worldPosition.xyz - cameraPosition );
		}
		vec3 worldNormal = transformNormalByInverseViewMatrix( transformedNormal, viewMatrix );
		#ifdef ENVMAP_MODE_REFLECTION
			vReflect = reflect( cameraToVertex, worldNormal );
		#else
			vReflect = refract( cameraToVertex, worldNormal, refractionRatio );
		#endif
	#endif
#endif`,Xu=`#ifdef USE_FOG
	vFogDepth = - mvPosition.z;
#endif`,qu=`#ifdef USE_FOG
	varying float vFogDepth;
#endif`,Yu=`#ifdef USE_FOG
	#ifdef FOG_EXP2
		float fogFactor = 1.0 - exp( - fogDensity * fogDensity * vFogDepth * vFogDepth );
	#else
		float fogFactor = smoothstep( fogNear, fogFar, vFogDepth );
	#endif
	gl_FragColor.rgb = mix( gl_FragColor.rgb, fogColor, fogFactor );
#endif`,Zu=`#ifdef USE_FOG
	uniform vec3 fogColor;
	varying float vFogDepth;
	#ifdef FOG_EXP2
		uniform float fogDensity;
	#else
		uniform float fogNear;
		uniform float fogFar;
	#endif
#endif`,Ju=`#ifdef USE_GRADIENTMAP
	uniform sampler2D gradientMap;
#endif
vec3 getGradientIrradiance( vec3 normal, vec3 lightDirection ) {
	float dotNL = dot( normal, lightDirection );
	vec2 coord = vec2( dotNL * 0.5 + 0.5, 0.0 );
	#ifdef USE_GRADIENTMAP
		return vec3( texture2D( gradientMap, coord ).r );
	#else
		vec2 fw = fwidth( coord ) * 0.5;
		return mix( vec3( 0.7 ), vec3( 1.0 ), smoothstep( 0.7 - fw.x, 0.7 + fw.x, coord.x ) );
	#endif
}`,$u=`#ifdef USE_LIGHTMAP
	uniform sampler2D lightMap;
	uniform float lightMapIntensity;
#endif`,Ku=`LambertMaterial material;
material.diffuseColor = diffuseColor.rgb;
material.specularStrength = specularStrength;`,Qu=`varying vec3 vViewPosition;
struct LambertMaterial {
	vec3 diffuseColor;
	float specularStrength;
};
void RE_Direct_Lambert( const in IncidentLight directLight, const in vec3 geometryPosition, const in vec3 geometryNormal, const in vec3 geometryViewDir, const in vec3 geometryClearcoatNormal, const in LambertMaterial material, inout ReflectedLight reflectedLight ) {
	float dotNL = saturate( dot( geometryNormal, directLight.direction ) );
	vec3 irradiance = dotNL * directLight.color;
	reflectedLight.directDiffuse += irradiance * BRDF_Lambert( material.diffuseColor );
}
void RE_IndirectDiffuse_Lambert( const in vec3 irradiance, const in vec3 geometryPosition, const in vec3 geometryNormal, const in vec3 geometryViewDir, const in vec3 geometryClearcoatNormal, const in LambertMaterial material, inout ReflectedLight reflectedLight ) {
	reflectedLight.indirectDiffuse += irradiance * BRDF_Lambert( material.diffuseColor );
}
#define RE_Direct				RE_Direct_Lambert
#define RE_IndirectDiffuse		RE_IndirectDiffuse_Lambert`,ju=`uniform bool receiveShadow;
uniform vec3 ambientLightColor;
#if defined( USE_LIGHT_PROBES )
	uniform vec3 lightProbe[ 9 ];
#endif
vec3 shGetIrradianceAt( in vec3 normal, in vec3 shCoefficients[ 9 ] ) {
	float x = normal.x, y = normal.y, z = normal.z;
	vec3 result = shCoefficients[ 0 ] * 0.886227;
	result += shCoefficients[ 1 ] * 2.0 * 0.511664 * y;
	result += shCoefficients[ 2 ] * 2.0 * 0.511664 * z;
	result += shCoefficients[ 3 ] * 2.0 * 0.511664 * x;
	result += shCoefficients[ 4 ] * 2.0 * 0.429043 * x * y;
	result += shCoefficients[ 5 ] * 2.0 * 0.429043 * y * z;
	result += shCoefficients[ 6 ] * ( 0.743125 * z * z - 0.247708 );
	result += shCoefficients[ 7 ] * 2.0 * 0.429043 * x * z;
	result += shCoefficients[ 8 ] * 0.429043 * ( x * x - y * y );
	return result;
}
vec3 getLightProbeIrradiance( const in vec3 lightProbe[ 9 ], const in vec3 normal ) {
	vec3 worldNormal = transformNormalByInverseViewMatrix( normal, viewMatrix );
	vec3 irradiance = shGetIrradianceAt( worldNormal, lightProbe );
	return irradiance;
}
vec3 getAmbientLightIrradiance( const in vec3 ambientLightColor ) {
	vec3 irradiance = ambientLightColor;
	return irradiance;
}
float getDistanceAttenuation( const in float lightDistance, const in float cutoffDistance, const in float decayExponent ) {
	float distanceFalloff = 1.0 / max( pow( lightDistance, decayExponent ), 0.01 );
	if ( cutoffDistance > 0.0 ) {
		distanceFalloff *= pow2( saturate( 1.0 - pow4( lightDistance / cutoffDistance ) ) );
	}
	return distanceFalloff;
}
float getSpotAttenuation( const in float coneCosine, const in float penumbraCosine, const in float angleCosine ) {
	return smoothstep( coneCosine, penumbraCosine, angleCosine );
}
#if NUM_SUN_LIGHTS > 0
	struct SunLight {
		vec3 direction;
		vec3 color;
	};
	uniform SunLight sunLights[ NUM_SUN_LIGHTS ];
	void getSunLightInfo( const in SunLight sunLight, out IncidentLight light ) {
		light.color = sunLight.color;
		light.direction = sunLight.direction;
		light.visible = true;
	}
#endif
#if NUM_DIR_LIGHTS > 0
	struct DirectionalLight {
		vec3 direction;
		vec3 color;
	};
	uniform DirectionalLight directionalLights[ NUM_DIR_LIGHTS ];
	void getDirectionalLightInfo( const in DirectionalLight directionalLight, out IncidentLight light ) {
		light.color = directionalLight.color;
		light.direction = directionalLight.direction;
		light.visible = true;
	}
#endif
#if NUM_POINT_LIGHTS > 0
	struct PointLight {
		vec3 position;
		vec3 color;
		float distance;
		float decay;
	};
	uniform PointLight pointLights[ NUM_POINT_LIGHTS ];
	void getPointLightInfo( const in PointLight pointLight, const in vec3 geometryPosition, out IncidentLight light ) {
		vec3 lVector = pointLight.position - geometryPosition;
		light.direction = normalize( lVector );
		float lightDistance = length( lVector );
		light.color = pointLight.color;
		light.color *= getDistanceAttenuation( lightDistance, pointLight.distance, pointLight.decay );
		light.visible = ( light.color != vec3( 0.0 ) );
	}
#endif
#if NUM_SPOT_LIGHTS > 0
	struct SpotLight {
		vec3 position;
		vec3 direction;
		vec3 color;
		float distance;
		float decay;
		float coneCos;
		float penumbraCos;
	};
	uniform SpotLight spotLights[ NUM_SPOT_LIGHTS ];
	void getSpotLightInfo( const in SpotLight spotLight, const in vec3 geometryPosition, out IncidentLight light ) {
		vec3 lVector = spotLight.position - geometryPosition;
		light.direction = normalize( lVector );
		float angleCos = dot( light.direction, spotLight.direction );
		float spotAttenuation = getSpotAttenuation( spotLight.coneCos, spotLight.penumbraCos, angleCos );
		if ( spotAttenuation > 0.0 ) {
			float lightDistance = length( lVector );
			light.color = spotLight.color * spotAttenuation;
			light.color *= getDistanceAttenuation( lightDistance, spotLight.distance, spotLight.decay );
			light.visible = ( light.color != vec3( 0.0 ) );
		} else {
			light.color = vec3( 0.0 );
			light.visible = false;
		}
	}
#endif
#if NUM_RECT_AREA_LIGHTS > 0
	struct RectAreaLight {
		vec3 color;
		vec3 position;
		vec3 halfWidth;
		vec3 halfHeight;
	};
	uniform sampler2D ltc_1;	uniform sampler2D ltc_2;
	uniform RectAreaLight rectAreaLights[ NUM_RECT_AREA_LIGHTS ];
#endif
#if NUM_HEMI_LIGHTS > 0
	struct HemisphereLight {
		vec3 direction;
		vec3 skyColor;
		vec3 groundColor;
	};
	uniform HemisphereLight hemisphereLights[ NUM_HEMI_LIGHTS ];
	vec3 getHemisphereLightIrradiance( const in HemisphereLight hemiLight, const in vec3 normal ) {
		float dotNL = dot( normal, hemiLight.direction );
		float hemiDiffuseWeight = 0.5 * dotNL + 0.5;
		vec3 irradiance = mix( hemiLight.groundColor, hemiLight.skyColor, hemiDiffuseWeight );
		return irradiance;
	}
#endif
#include <lightprobes_pars_fragment>`,td=`#ifdef USE_ENVMAP
	vec3 getIBLIrradiance( const in vec3 normal ) {
		#ifdef ENVMAP_TYPE_PMREM
			vec3 worldNormal = transformNormalByInverseViewMatrix( normal, viewMatrix );
			vec4 envMapColor = textureLod( envMap, envMapRotation * worldNormal, ENVMAP_MAX_LOD );
			return PI * envMapColor.rgb * envMapIntensity;
		#else
			return vec3( 0.0 );
		#endif
	}
	vec3 getIBLRadiance( const in vec3 viewDir, const in vec3 normal, const in float roughness ) {
		#ifdef ENVMAP_TYPE_PMREM
			vec3 reflectVec = reflect( - viewDir, normal );
			reflectVec = normalize( mix( reflectVec, normal, pow4( roughness ) ) );
			reflectVec = transformDirectionByInverseViewMatrix( reflectVec, viewMatrix );
			vec4 envMapColor = textureLod( envMap, envMapRotation * reflectVec, roughnessToMip( roughness ) );
			return envMapColor.rgb * envMapIntensity;
		#else
			return vec3( 0.0 );
		#endif
	}
	#ifdef USE_RETROREFLECTION
		vec3 getIBLRetroRadiance( const in vec3 viewDir, const in vec3 normal, const in float roughness ) {
			#ifdef ENVMAP_TYPE_PMREM
				vec3 retroVec = normalize( mix( viewDir, normal, pow4( roughness ) ) );
				retroVec = transformDirectionByInverseViewMatrix( retroVec, viewMatrix );
				vec4 envMapColor = textureLod( envMap, envMapRotation * retroVec, roughnessToMip( roughness ) );
				return envMapColor.rgb * envMapIntensity;
			#else
				return vec3( 0.0 );
			#endif
		}
	#endif
	#ifdef USE_ANISOTROPY
		vec3 getIBLAnisotropyRadiance( const in vec3 viewDir, const in vec3 normal, const in float roughness, const in vec3 bitangent, const in float anisotropy ) {
			#ifdef ENVMAP_TYPE_PMREM
				vec3 bentNormal = cross( bitangent, viewDir );
				bentNormal = normalize( cross( bentNormal, bitangent ) );
				bentNormal = normalize( mix( bentNormal, normal, pow2( pow2( 1.0 - anisotropy * ( 1.0 - roughness ) ) ) ) );
				return getIBLRadiance( viewDir, bentNormal, roughness );
			#else
				return vec3( 0.0 );
			#endif
		}
		#ifdef USE_RETROREFLECTION
			vec3 getIBLAnisotropyRetroRadiance( const in vec3 viewDir, const in vec3 normal, const in float roughness, const in vec3 bitangent, const in float anisotropy ) {
				#ifdef ENVMAP_TYPE_PMREM
					vec3 bentNormal = cross( bitangent, viewDir );
					bentNormal = normalize( cross( bentNormal, bitangent ) );
					bentNormal = normalize( mix( bentNormal, normal, pow2( pow2( 1.0 - anisotropy * ( 1.0 - roughness ) ) ) ) );
					return getIBLRetroRadiance( viewDir, bentNormal, roughness );
				#else
					return vec3( 0.0 );
				#endif
			}
		#endif
	#endif
#endif`,ed=`ToonMaterial material;
material.diffuseColor = diffuseColor.rgb;`,nd=`varying vec3 vViewPosition;
struct ToonMaterial {
	vec3 diffuseColor;
};
void RE_Direct_Toon( const in IncidentLight directLight, const in vec3 geometryPosition, const in vec3 geometryNormal, const in vec3 geometryViewDir, const in vec3 geometryClearcoatNormal, const in ToonMaterial material, inout ReflectedLight reflectedLight ) {
	vec3 irradiance = getGradientIrradiance( geometryNormal, directLight.direction ) * directLight.color;
	reflectedLight.directDiffuse += irradiance * BRDF_Lambert( material.diffuseColor );
}
void RE_IndirectDiffuse_Toon( const in vec3 irradiance, const in vec3 geometryPosition, const in vec3 geometryNormal, const in vec3 geometryViewDir, const in vec3 geometryClearcoatNormal, const in ToonMaterial material, inout ReflectedLight reflectedLight ) {
	reflectedLight.indirectDiffuse += irradiance * BRDF_Lambert( material.diffuseColor );
}
#define RE_Direct				RE_Direct_Toon
#define RE_IndirectDiffuse		RE_IndirectDiffuse_Toon`,id=`BlinnPhongMaterial material;
material.diffuseColor = diffuseColor.rgb;
material.specularColor = specular;
material.specularShininess = shininess;
material.specularStrength = specularStrength;`,sd=`varying vec3 vViewPosition;
struct BlinnPhongMaterial {
	vec3 diffuseColor;
	vec3 specularColor;
	float specularShininess;
	float specularStrength;
};
void RE_Direct_BlinnPhong( const in IncidentLight directLight, const in vec3 geometryPosition, const in vec3 geometryNormal, const in vec3 geometryViewDir, const in vec3 geometryClearcoatNormal, const in BlinnPhongMaterial material, inout ReflectedLight reflectedLight ) {
	float dotNL = saturate( dot( geometryNormal, directLight.direction ) );
	vec3 irradiance = dotNL * directLight.color;
	reflectedLight.directDiffuse += irradiance * BRDF_Lambert( material.diffuseColor );
	reflectedLight.directSpecular += irradiance * BRDF_BlinnPhong( directLight.direction, geometryViewDir, geometryNormal, material.specularColor, material.specularShininess ) * material.specularStrength;
}
void RE_IndirectDiffuse_BlinnPhong( const in vec3 irradiance, const in vec3 geometryPosition, const in vec3 geometryNormal, const in vec3 geometryViewDir, const in vec3 geometryClearcoatNormal, const in BlinnPhongMaterial material, inout ReflectedLight reflectedLight ) {
	reflectedLight.indirectDiffuse += irradiance * BRDF_Lambert( material.diffuseColor );
}
#define RE_Direct				RE_Direct_BlinnPhong
#define RE_IndirectDiffuse		RE_IndirectDiffuse_BlinnPhong`,rd=`PhysicalMaterial material;
material.diffuseColor = diffuseColor.rgb;
material.diffuseContribution = diffuseColor.rgb * ( 1.0 - metalnessFactor );
material.metalness = metalnessFactor;
vec3 dxy = max( abs( dFdx( nonPerturbedNormal ) ), abs( dFdy( nonPerturbedNormal ) ) );
float geometryRoughness = max( max( dxy.x, dxy.y ), dxy.z );
material.roughness = max( roughnessFactor, 0.045 );
material.roughness += geometryRoughness;
material.roughness = min( material.roughness, 1.0 );
#ifdef USE_DIFFUSE_ROUGHNESS
	float diffuseRoughnessFactor = diffuseRoughness;
	#ifdef USE_DIFFUSE_ROUGHNESSMAP
		diffuseRoughnessFactor *= texture2D( diffuseRoughnessMap, vDiffuseRoughnessMapUv ).r;
	#endif
	material.diffuseRoughness = saturate( diffuseRoughnessFactor );
#endif
#ifdef IOR
	material.ior = ior;
	#ifdef USE_SPECULAR
		float specularIntensityFactor = specularIntensity;
		vec3 specularColorFactor = specularColor;
		#ifdef USE_SPECULAR_COLORMAP
			specularColorFactor *= texture2D( specularColorMap, vSpecularColorMapUv ).rgb;
		#endif
		#ifdef USE_SPECULAR_INTENSITYMAP
			specularIntensityFactor *= texture2D( specularIntensityMap, vSpecularIntensityMapUv ).a;
		#endif
		material.specularF90 = mix( specularIntensityFactor, 1.0, metalnessFactor );
	#else
		float specularIntensityFactor = 1.0;
		vec3 specularColorFactor = vec3( 1.0 );
		material.specularF90 = 1.0;
	#endif
	material.specularColor = min( pow2( ( material.ior - 1.0 ) / ( material.ior + 1.0 ) ) * specularColorFactor, vec3( 1.0 ) ) * specularIntensityFactor;
	material.specularColorBlended = mix( material.specularColor, diffuseColor.rgb, metalnessFactor );
#else
	material.specularColor = vec3( 0.04 );
	material.specularColorBlended = mix( material.specularColor, diffuseColor.rgb, metalnessFactor );
	material.specularF90 = 1.0;
#endif
#ifdef USE_CLEARCOAT
	material.clearcoat = clearcoat;
	material.clearcoatRoughness = clearcoatRoughness;
	material.clearcoatF0 = vec3( 0.04 );
	material.clearcoatF90 = 1.0;
	#ifdef USE_CLEARCOATMAP
		material.clearcoat *= texture2D( clearcoatMap, vClearcoatMapUv ).x;
	#endif
	#ifdef USE_CLEARCOAT_ROUGHNESSMAP
		material.clearcoatRoughness *= texture2D( clearcoatRoughnessMap, vClearcoatRoughnessMapUv ).y;
	#endif
	material.clearcoat = saturate( material.clearcoat );	material.clearcoatRoughness = max( material.clearcoatRoughness, 0.045 );
	material.clearcoatRoughness += geometryRoughness;
	material.clearcoatRoughness = min( material.clearcoatRoughness, 1.0 );
#endif
#ifdef USE_DISPERSION
	material.dispersion = dispersion;
#endif
#ifdef USE_RETROREFLECTION
	material.retroreflectivity = retroreflectivity;
#endif
#ifdef USE_IRIDESCENCE
	material.iridescence = iridescence;
	material.iridescenceIOR = iridescenceIOR;
	#ifdef USE_IRIDESCENCEMAP
		material.iridescence *= texture2D( iridescenceMap, vIridescenceMapUv ).r;
	#endif
	#ifdef USE_IRIDESCENCE_THICKNESSMAP
		material.iridescenceThickness = (iridescenceThicknessMaximum - iridescenceThicknessMinimum) * texture2D( iridescenceThicknessMap, vIridescenceThicknessMapUv ).g + iridescenceThicknessMinimum;
	#else
		material.iridescenceThickness = iridescenceThicknessMaximum;
	#endif
#endif
#ifdef USE_SHEEN
	material.sheenColor = sheenColor;
	#ifdef USE_SHEEN_COLORMAP
		material.sheenColor *= texture2D( sheenColorMap, vSheenColorMapUv ).rgb;
	#endif
	material.sheenRoughness = clamp( sheenRoughness, 0.0001, 1.0 );
	#ifdef USE_SHEEN_ROUGHNESSMAP
		material.sheenRoughness *= texture2D( sheenRoughnessMap, vSheenRoughnessMapUv ).a;
	#endif
#endif
#ifdef USE_ANISOTROPY
	#ifdef USE_ANISOTROPYMAP
		mat2 anisotropyMat = mat2( anisotropyVector.x, anisotropyVector.y, - anisotropyVector.y, anisotropyVector.x );
		vec3 anisotropyPolar = texture2D( anisotropyMap, vAnisotropyMapUv ).rgb;
		vec2 anisotropyV = anisotropyMat * normalize( 2.0 * anisotropyPolar.rg - vec2( 1.0 ) ) * anisotropyPolar.b;
	#else
		vec2 anisotropyV = anisotropyVector;
	#endif
	material.anisotropy = length( anisotropyV );
	if( material.anisotropy == 0.0 ) {
		anisotropyV = vec2( 1.0, 0.0 );
	} else {
		anisotropyV /= material.anisotropy;
		material.anisotropy = saturate( material.anisotropy );
	}
	material.alphaT = mix( pow2( material.roughness ), 1.0, pow2( material.anisotropy ) );
	material.anisotropyT = tbn[ 0 ] * anisotropyV.x + tbn[ 1 ] * anisotropyV.y;
	material.anisotropyB = tbn[ 1 ] * anisotropyV.x - tbn[ 0 ] * anisotropyV.y;
#endif`,ad=`uniform sampler2D dfgLUT;
struct PhysicalMaterial {
	vec3 diffuseColor;
	vec3 diffuseContribution;
	vec3 specularColor;
	vec3 specularColorBlended;
	float roughness;
	float metalness;
	float specularF90;
	float dispersion;
	vec2 dfg;
	vec3 multiScatteringCompensation;
	#ifdef USE_DIFFUSE_ROUGHNESS
		float diffuseRoughness;
	#endif
	#ifdef USE_RETROREFLECTION
		float retroreflectivity;
	#endif
	#ifdef USE_CLEARCOAT
		float clearcoat;
		float clearcoatRoughness;
		vec3 clearcoatF0;
		float clearcoatF90;
	#endif
	#ifdef USE_IRIDESCENCE
		float iridescence;
		float iridescenceIOR;
		float iridescenceThickness;
		vec3 iridescenceFresnel;
		vec3 iridescenceF0Dielectric;
		vec3 iridescenceF0Metallic;
	#endif
	#ifdef USE_SHEEN
		vec3 sheenColor;
		float sheenRoughness;
	#endif
	#ifdef IOR
		float ior;
	#endif
	#ifdef USE_TRANSMISSION
		float transmission;
		float transmissionAlpha;
		float thickness;
		float attenuationDistance;
		vec3 attenuationColor;
	#endif
	#ifdef USE_ANISOTROPY
		float anisotropy;
		float alphaT;
		vec3 anisotropyT;
		vec3 anisotropyB;
	#endif
};
vec3 clearcoatSpecularDirect = vec3( 0.0 );
vec3 clearcoatSpecularIndirect = vec3( 0.0 );
vec3 sheenSpecularDirect = vec3( 0.0 );
vec3 sheenSpecularIndirect = vec3(0.0 );
vec3 Schlick_to_F0( const in vec3 f, const in float f90, const in float dotVH ) {
    float x = clamp( 1.0 - dotVH, 0.0, 1.0 );
    float x2 = x * x;
    float x5 = clamp( x * x2 * x2, 0.0, 0.9999 );
    return ( f - vec3( f90 ) * x5 ) / ( 1.0 - x5 );
}
#ifdef USE_DIFFUSE_ROUGHNESS
	const float EON_EPSILON = 1e-7;
	float FON_DirectionalAlbedo( const in float mu, const in float roughness, const in float A ) {
		float muComp = 1.0 - mu;
		float gOverPi = muComp * ( 0.0571085289 + muComp * ( 0.491881867 + muComp * ( - 0.332181442 + muComp * 0.0714429953 ) ) );
		return A * ( 1.0 + roughness * gOverPi );
	}
	vec3 BRDF_EON( const in vec3 lightDir, const in vec3 viewDir, const in vec3 normal, const in vec3 diffuseColor, const in float roughness ) {
		vec3 rho = saturate( diffuseColor );
		if ( roughness <= EON_EPSILON ) return BRDF_Lambert( rho );
		float dotNL = saturate( dot( normal, lightDir ) );
		float dotNV = saturate( dot( normal, viewDir ) );
		float s = dot( lightDir, viewDir ) - dotNL * dotNV;
		float sOverT = ( s > 0.0 ) ? s / max( max( dotNL, dotNV ), EON_EPSILON ) : s;
		float A = 1.0 / ( 1.0 + ( 0.5 - 2.0 / ( 3.0 * PI ) ) * roughness );
		vec3 singleScatter = rho * RECIPROCAL_PI * A * ( 1.0 + roughness * sOverT );
		float averageAlbedo = A * ( 1.0 + ( 2.0 / 3.0 - 28.0 / ( 15.0 * PI ) ) * roughness );
		float albedoV = FON_DirectionalAlbedo( dotNV, roughness, A );
		float albedoL = FON_DirectionalAlbedo( dotNL, roughness, A );
		vec3 rhoMultiScatter = rho * rho * averageAlbedo / max( vec3( EON_EPSILON ), vec3( 1.0 ) - rho * ( 1.0 - averageAlbedo ) );
		vec3 multiScatter = rhoMultiScatter * RECIPROCAL_PI
			* max( EON_EPSILON, 1.0 - albedoV )
			* max( EON_EPSILON, 1.0 - albedoL )
			/ max( EON_EPSILON, 1.0 - averageAlbedo );
		return singleScatter + multiScatter;
	}
	vec3 EON_DirectionalAlbedo( const in vec3 diffuseColor, const in float roughness, const in float dotNV ) {
		vec3 rho = saturate( diffuseColor );
		if ( roughness <= EON_EPSILON ) return rho;
		float A = 1.0 / ( 1.0 + ( 0.5 - 2.0 / ( 3.0 * PI ) ) * roughness );
		float directionalAlbedo = FON_DirectionalAlbedo( dotNV, roughness, A );
		float averageAlbedo = A * ( 1.0 + ( 2.0 / 3.0 - 28.0 / ( 15.0 * PI ) ) * roughness );
		vec3 rhoMultiScatter = rho * rho * averageAlbedo / max( vec3( EON_EPSILON ), vec3( 1.0 ) - rho * ( 1.0 - averageAlbedo ) );
		return rho * directionalAlbedo + rhoMultiScatter * ( 1.0 - directionalAlbedo );
	}
#endif
float V_GGX_SmithCorrelated( const in float alpha, const in float dotNL, const in float dotNV ) {
	float a2 = pow2( alpha );
	float gv = dotNL * sqrt( a2 + ( 1.0 - a2 ) * pow2( dotNV ) );
	float gl = dotNV * sqrt( a2 + ( 1.0 - a2 ) * pow2( dotNL ) );
	return 0.5 / max( gv + gl, EPSILON );
}
float D_GGX( const in float alpha, const in float dotNH ) {
	float a2 = pow2( alpha );
	float denom = pow2( dotNH ) * ( a2 - 1.0 ) + 1.0;
	return RECIPROCAL_PI * a2 / pow2( denom );
}
#ifdef USE_ANISOTROPY
	float V_GGX_SmithCorrelated_Anisotropic( const in float alphaT, const in float alphaB, const in float dotTV, const in float dotBV, const in float dotTL, const in float dotBL, const in float dotNV, const in float dotNL ) {
		float gv = dotNL * length( vec3( alphaT * dotTV, alphaB * dotBV, dotNV ) );
		float gl = dotNV * length( vec3( alphaT * dotTL, alphaB * dotBL, dotNL ) );
		return 0.5 / max( gv + gl, EPSILON );
	}
	float D_GGX_Anisotropic( const in float alphaT, const in float alphaB, const in float dotNH, const in float dotTH, const in float dotBH ) {
		float a2 = alphaT * alphaB;
		highp vec3 v = vec3( alphaB * dotTH, alphaT * dotBH, a2 * dotNH );
		highp float v2 = dot( v, v );
		float w2 = a2 / v2;
		return RECIPROCAL_PI * a2 * pow2 ( w2 );
	}
#endif
#ifdef USE_CLEARCOAT
	vec3 BRDF_GGX_Clearcoat( const in vec3 lightDir, const in vec3 viewDir, const in vec3 normal, const in PhysicalMaterial material) {
		vec3 f0 = material.clearcoatF0;
		float f90 = material.clearcoatF90;
		float roughness = max( material.clearcoatRoughness, 0.045 );
		float alpha = pow2( roughness );
		vec3 halfDir = normalize( lightDir + viewDir );
		float dotNL = saturate( dot( normal, lightDir ) );
		float dotNV = saturate( dot( normal, viewDir ) );
		float dotNH = saturate( dot( normal, halfDir ) );
		float dotVH = saturate( dot( viewDir, halfDir ) );
		vec3 F = F_Schlick( f0, f90, dotVH );
		float V = V_GGX_SmithCorrelated( alpha, dotNL, dotNV );
		float D = D_GGX( alpha, dotNH );
		return F * ( V * D );
	}
#endif
vec3 BRDF_GGX( const in vec3 lightDir, const in vec3 viewDir, const in vec3 normal, const in PhysicalMaterial material ) {
	vec3 f0 = material.specularColorBlended;
	float f90 = material.specularF90;
	float roughness = max( material.roughness, 0.045 );
	float alpha = pow2( roughness );
	vec3 halfDir = normalize( lightDir + viewDir );
	float dotNL = saturate( dot( normal, lightDir ) );
	float dotNV = saturate( dot( normal, viewDir ) );
	float dotNH = saturate( dot( normal, halfDir ) );
	float dotVH = saturate( dot( viewDir, halfDir ) );
	vec3 F = F_Schlick( f0, f90, dotVH );
	#ifdef USE_IRIDESCENCE
		F = mix( F, material.iridescenceFresnel, material.iridescence );
	#endif
	#ifdef USE_ANISOTROPY
		float dotTL = dot( material.anisotropyT, lightDir );
		float dotTV = dot( material.anisotropyT, viewDir );
		float dotTH = dot( material.anisotropyT, halfDir );
		float dotBL = dot( material.anisotropyB, lightDir );
		float dotBV = dot( material.anisotropyB, viewDir );
		float dotBH = dot( material.anisotropyB, halfDir );
		float alphaT = max( material.alphaT, alpha );
		float V = V_GGX_SmithCorrelated_Anisotropic( alphaT, alpha, dotTV, dotBV, dotTL, dotBL, dotNV, dotNL );
		float D = D_GGX_Anisotropic( alphaT, alpha, dotNH, dotTH, dotBH );
	#else
		float V = V_GGX_SmithCorrelated( alpha, dotNL, dotNV );
		float D = D_GGX( alpha, dotNH );
	#endif
	return F * ( V * D );
}
vec2 LTC_Uv( const in vec3 N, const in vec3 V, const in float roughness ) {
	const float LUT_SIZE = 64.0;
	const float LUT_SCALE = ( LUT_SIZE - 1.0 ) / LUT_SIZE;
	const float LUT_BIAS = 0.5 / LUT_SIZE;
	float dotNV = saturate( dot( N, V ) );
	vec2 uv = vec2( roughness, sqrt( 1.0 - dotNV ) );
	uv = uv * LUT_SCALE + LUT_BIAS;
	return uv;
}
float LTC_ClippedSphereFormFactor( const in vec3 f ) {
	float l = length( f );
	return max( ( l * l + f.z ) / ( l + 1.0 ), 0.0 );
}
vec3 LTC_EdgeVectorFormFactor( const in vec3 v1, const in vec3 v2 ) {
	float x = dot( v1, v2 );
	float y = abs( x );
	float a = 0.8543985 + ( 0.4965155 + 0.0145206 * y ) * y;
	float b = 3.4175940 + ( 4.1616724 + y ) * y;
	float v = a / b;
	float theta_sintheta = ( x > 0.0 ) ? v : 0.5 * inversesqrt( max( 1.0 - x * x, 1e-7 ) ) - v;
	return cross( v1, v2 ) * theta_sintheta;
}
vec3 LTC_Evaluate( const in vec3 N, const in vec3 V, const in vec3 P, const in mat3 mInv, const in vec3 rectCoords[ 4 ] ) {
	vec3 v1 = rectCoords[ 1 ] - rectCoords[ 0 ];
	vec3 v2 = rectCoords[ 3 ] - rectCoords[ 0 ];
	vec3 lightNormal = cross( v1, v2 );
	if( dot( lightNormal, P - rectCoords[ 0 ] ) < 0.0 ) return vec3( 0.0 );
	vec3 T1, T2;
	T1 = normalize( V - N * dot( V, N ) );
	T2 = - cross( N, T1 );
	mat3 mat = mInv * transpose( mat3( T1, T2, N ) );
	vec3 coords[ 4 ];
	coords[ 0 ] = mat * ( rectCoords[ 0 ] - P );
	coords[ 1 ] = mat * ( rectCoords[ 1 ] - P );
	coords[ 2 ] = mat * ( rectCoords[ 2 ] - P );
	coords[ 3 ] = mat * ( rectCoords[ 3 ] - P );
	coords[ 0 ] = normalize( coords[ 0 ] );
	coords[ 1 ] = normalize( coords[ 1 ] );
	coords[ 2 ] = normalize( coords[ 2 ] );
	coords[ 3 ] = normalize( coords[ 3 ] );
	vec3 vectorFormFactor = vec3( 0.0 );
	vectorFormFactor += LTC_EdgeVectorFormFactor( coords[ 0 ], coords[ 1 ] );
	vectorFormFactor += LTC_EdgeVectorFormFactor( coords[ 1 ], coords[ 2 ] );
	vectorFormFactor += LTC_EdgeVectorFormFactor( coords[ 2 ], coords[ 3 ] );
	vectorFormFactor += LTC_EdgeVectorFormFactor( coords[ 3 ], coords[ 0 ] );
	float result = LTC_ClippedSphereFormFactor( vectorFormFactor );
	return vec3( result );
}
#if defined( USE_SHEEN )
float D_Charlie( float roughness, float dotNH ) {
	float alpha = pow2( roughness );
	float invAlpha = 1.0 / alpha;
	float cos2h = dotNH * dotNH;
	float sin2h = max( 1.0 - cos2h, 0.0078125 );
	return ( 2.0 + invAlpha ) * pow( sin2h, invAlpha * 0.5 ) / ( 2.0 * PI );
}
float V_Neubelt( float dotNV, float dotNL ) {
	return saturate( 1.0 / ( 4.0 * ( dotNL + dotNV - dotNL * dotNV ) ) );
}
vec3 BRDF_Sheen( const in vec3 lightDir, const in vec3 viewDir, const in vec3 normal, vec3 sheenColor, const in float sheenRoughness ) {
	vec3 halfDir = normalize( lightDir + viewDir );
	float dotNL = saturate( dot( normal, lightDir ) );
	float dotNV = saturate( dot( normal, viewDir ) );
	float dotNH = saturate( dot( normal, halfDir ) );
	float D = D_Charlie( sheenRoughness, dotNH );
	float V = V_Neubelt( dotNV, dotNL );
	return sheenColor * ( D * V );
}
#endif
float IBLSheenBRDF( const in vec3 normal, const in vec3 viewDir, const in float roughness ) {
	float dotNV = saturate( dot( normal, viewDir ) );
	float r2 = roughness * roughness;
	float rInv = 1.0 / ( roughness + 0.1 );
	float a = -1.9362 + 1.0678 * roughness + 0.4573 * r2 - 0.8469 * rInv;
	float b = -0.6014 + 0.5538 * roughness - 0.4670 * r2 - 0.1255 * rInv;
	float DG = exp( a * dotNV + b );
	return saturate( DG );
}
vec3 EnvironmentBRDF( const in vec3 normal, const in vec3 viewDir, const in vec3 specularColor, const in float specularF90, const in float roughness ) {
	float dotNV = saturate( dot( normal, viewDir ) );
	vec2 fab = texture2D( dfgLUT, vec2( roughness, dotNV ) ).rg;
	return specularColor * fab.x + specularF90 * fab.y;
}
#ifdef USE_IRIDESCENCE
void computeMultiscatteringIridescence( const in vec2 fab, const in vec3 specularColor, const in float specularF90, const in float iridescence, const in vec3 iridescenceF0, inout vec3 singleScatter, inout vec3 multiScatter ) {
#else
void computeMultiscattering( const in vec2 fab, const in vec3 specularColor, const in float specularF90, inout vec3 singleScatter, inout vec3 multiScatter ) {
#endif
	#ifdef USE_IRIDESCENCE
		vec3 Fr = mix( specularColor, iridescenceF0, iridescence );
	#else
		vec3 Fr = specularColor;
	#endif
	vec3 FssEss = Fr * fab.x + specularF90 * fab.y;
	float Ess = fab.x + fab.y;
	float Ems = 1.0 - Ess;
	vec3 Favg = Fr + ( 1.0 - Fr ) * 0.047619;	vec3 Fms = FssEss * Favg / ( 1.0 - Ems * Favg );
	singleScatter += FssEss;
	multiScatter += Fms * Ems;
}
#if NUM_RECT_AREA_LIGHTS > 0
	void RE_Direct_RectArea_Physical( const in RectAreaLight rectAreaLight, const in vec3 geometryPosition, const in vec3 geometryNormal, const in vec3 geometryViewDir, const in vec3 geometryClearcoatNormal, const in PhysicalMaterial material, inout ReflectedLight reflectedLight ) {
		vec3 normal = geometryNormal;
		vec3 viewDir = geometryViewDir;
		vec3 position = geometryPosition;
		vec3 lightPos = rectAreaLight.position;
		vec3 halfWidth = rectAreaLight.halfWidth;
		vec3 halfHeight = rectAreaLight.halfHeight;
		vec3 lightColor = rectAreaLight.color;
		float roughness = material.roughness;
		vec3 rectCoords[ 4 ];
		rectCoords[ 0 ] = lightPos + halfWidth - halfHeight;		rectCoords[ 1 ] = lightPos - halfWidth - halfHeight;
		rectCoords[ 2 ] = lightPos - halfWidth + halfHeight;
		rectCoords[ 3 ] = lightPos + halfWidth + halfHeight;
		vec2 uv = LTC_Uv( normal, viewDir, roughness );
		vec4 t1 = texture2D( ltc_1, uv );
		vec4 t2 = texture2D( ltc_2, uv );
		mat3 mInv = mat3(
			vec3( t1.x, 0, t1.y ),
			vec3(    0, 1,    0 ),
			vec3( t1.z, 0, t1.w )
		);
		vec3 fresnel = ( material.specularColorBlended * t2.x + ( material.specularF90 - material.specularColorBlended ) * t2.y );
		reflectedLight.directSpecular += lightColor * fresnel * LTC_Evaluate( normal, viewDir, position, mInv, rectCoords );
		reflectedLight.directDiffuse += lightColor * material.diffuseContribution * LTC_Evaluate( normal, viewDir, position, mat3( 1.0 ), rectCoords );
		#ifdef USE_CLEARCOAT
			vec3 Ncc = geometryClearcoatNormal;
			vec2 uvClearcoat = LTC_Uv( Ncc, viewDir, material.clearcoatRoughness );
			vec4 t1Clearcoat = texture2D( ltc_1, uvClearcoat );
			vec4 t2Clearcoat = texture2D( ltc_2, uvClearcoat );
			mat3 mInvClearcoat = mat3(
				vec3( t1Clearcoat.x, 0, t1Clearcoat.y ),
				vec3(             0, 1,             0 ),
				vec3( t1Clearcoat.z, 0, t1Clearcoat.w )
			);
			vec3 fresnelClearcoat = material.clearcoatF0 * t2Clearcoat.x + ( material.clearcoatF90 - material.clearcoatF0 ) * t2Clearcoat.y;
			clearcoatSpecularDirect += lightColor * fresnelClearcoat * LTC_Evaluate( Ncc, viewDir, position, mInvClearcoat, rectCoords );
		#endif
	}
#endif
void RE_Direct_Physical( const in IncidentLight directLight, const in vec3 geometryPosition, const in vec3 geometryNormal, const in vec3 geometryViewDir, const in vec3 geometryClearcoatNormal, const in PhysicalMaterial material, inout ReflectedLight reflectedLight ) {
	float dotNL = saturate( dot( geometryNormal, directLight.direction ) );
	vec3 irradiance = dotNL * directLight.color;
	#ifdef USE_CLEARCOAT
		float dotNLcc = saturate( dot( geometryClearcoatNormal, directLight.direction ) );
		vec3 ccIrradiance = dotNLcc * directLight.color;
		clearcoatSpecularDirect += ccIrradiance * BRDF_GGX_Clearcoat( directLight.direction, geometryViewDir, geometryClearcoatNormal, material );
	#endif
	#ifdef USE_SHEEN
 
 		sheenSpecularDirect += irradiance * BRDF_Sheen( directLight.direction, geometryViewDir, geometryNormal, material.sheenColor, material.sheenRoughness );
 
 		float sheenAlbedoV = IBLSheenBRDF( geometryNormal, geometryViewDir, material.sheenRoughness );
 		float sheenAlbedoL = IBLSheenBRDF( geometryNormal, directLight.direction, material.sheenRoughness );
 
 		float sheenEnergyComp = 1.0 - max3( material.sheenColor ) * max( sheenAlbedoV, sheenAlbedoL );
 
 		irradiance *= sheenEnergyComp;
 
 	#endif
	vec3 specularBRDF = BRDF_GGX( directLight.direction, geometryViewDir, geometryNormal, material );
	#ifdef USE_RETROREFLECTION
		vec3 retroViewDir = reflect( - geometryViewDir, geometryNormal );
		vec3 retroSpecularBRDF = BRDF_GGX( directLight.direction, retroViewDir, geometryNormal, material );
		specularBRDF = mix( specularBRDF, retroSpecularBRDF, saturate( material.retroreflectivity ) );
	#endif
	reflectedLight.directSpecular += irradiance * specularBRDF * material.multiScatteringCompensation;
	vec3 halfDir = normalize( directLight.direction + geometryViewDir );
	float dotVH = saturate( dot( geometryViewDir, halfDir ) );
	vec3 F = F_Schlick( material.specularColor, material.specularF90, dotVH );
	#ifdef USE_RETROREFLECTION
		vec3 retroHalfDir = normalize( directLight.direction + retroViewDir );
		float dotRetroVH = saturate( dot( retroViewDir, retroHalfDir ) );
		vec3 retroF = F_Schlick( material.specularColor, material.specularF90, dotRetroVH );
		F = mix( F, retroF, saturate( material.retroreflectivity ) );
	#endif
	#ifdef USE_DIFFUSE_ROUGHNESS
		vec3 diffuseBRDF = BRDF_EON( directLight.direction, geometryViewDir, geometryNormal, material.diffuseColor, material.diffuseRoughness ) * ( 1.0 - material.metalness );
	#else
		vec3 diffuseBRDF = BRDF_Lambert( material.diffuseContribution );
	#endif
	reflectedLight.directDiffuse += irradiance * diffuseBRDF * ( 1.0 - F );
}
void RE_IndirectDiffuse_Physical( const in vec3 irradiance, const in vec3 geometryPosition, const in vec3 geometryNormal, const in vec3 geometryViewDir, const in vec3 geometryClearcoatNormal, const in PhysicalMaterial material, inout ReflectedLight reflectedLight ) {
	vec3 singleScattering = vec3( 0.0 );
	vec3 multiScattering = vec3( 0.0 );
	#ifdef USE_IRIDESCENCE
		computeMultiscatteringIridescence( material.dfg, material.specularColor, material.specularF90, material.iridescence, material.iridescenceF0Dielectric, singleScattering, multiScattering );
	#else
		computeMultiscattering( material.dfg, material.specularColor, material.specularF90, singleScattering, multiScattering );
	#endif
	#ifdef USE_DIFFUSE_ROUGHNESS
		float dotNV = saturate( dot( geometryNormal, geometryViewDir ) );
		vec3 diffuseAlbedo = EON_DirectionalAlbedo( material.diffuseColor, material.diffuseRoughness, dotNV ) * ( 1.0 - material.metalness );
		vec3 diffuse = irradiance * RECIPROCAL_PI * diffuseAlbedo * ( 1.0 - singleScattering - multiScattering );
	#else
		vec3 diffuse = irradiance * BRDF_Lambert( material.diffuseContribution ) * ( 1.0 - singleScattering - multiScattering );
	#endif
	#ifdef USE_SHEEN
		float sheenAlbedo = IBLSheenBRDF( geometryNormal, geometryViewDir, material.sheenRoughness );
		sheenSpecularIndirect += irradiance * material.sheenColor * sheenAlbedo * RECIPROCAL_PI;
		float sheenEnergyComp = 1.0 - max3( material.sheenColor ) * sheenAlbedo;
		diffuse *= sheenEnergyComp;
	#endif
	reflectedLight.indirectDiffuse += diffuse;
}
void RE_IndirectSpecular_Physical( const in vec3 radiance, const in vec3 irradiance, const in vec3 clearcoatRadiance, const in vec3 geometryPosition, const in vec3 geometryNormal, const in vec3 geometryViewDir, const in vec3 geometryClearcoatNormal, const in PhysicalMaterial material, inout ReflectedLight reflectedLight) {
	#ifdef USE_CLEARCOAT
		clearcoatSpecularIndirect += clearcoatRadiance * EnvironmentBRDF( geometryClearcoatNormal, geometryViewDir, material.clearcoatF0, material.clearcoatF90, material.clearcoatRoughness );
	#endif
	#ifdef USE_SHEEN
		sheenSpecularIndirect += irradiance * material.sheenColor * IBLSheenBRDF( geometryNormal, geometryViewDir, material.sheenRoughness ) * RECIPROCAL_PI;
 	#endif
	vec3 singleScatteringDielectric = vec3( 0.0 );
	vec3 multiScatteringDielectric = vec3( 0.0 );
	vec3 singleScatteringMetallic = vec3( 0.0 );
	vec3 multiScatteringMetallic = vec3( 0.0 );
	#ifdef USE_IRIDESCENCE
		computeMultiscatteringIridescence( material.dfg, material.specularColor, material.specularF90, material.iridescence, material.iridescenceF0Dielectric, singleScatteringDielectric, multiScatteringDielectric );
		computeMultiscatteringIridescence( material.dfg, material.diffuseColor, material.specularF90, material.iridescence, material.iridescenceF0Metallic, singleScatteringMetallic, multiScatteringMetallic );
	#else
		computeMultiscattering( material.dfg, material.specularColor, material.specularF90, singleScatteringDielectric, multiScatteringDielectric );
		computeMultiscattering( material.dfg, material.diffuseColor, material.specularF90, singleScatteringMetallic, multiScatteringMetallic );
	#endif
	vec3 singleScattering = mix( singleScatteringDielectric, singleScatteringMetallic, material.metalness );
	vec3 multiScattering = mix( multiScatteringDielectric, multiScatteringMetallic, material.metalness );
	vec3 totalScatteringDielectric = singleScatteringDielectric + multiScatteringDielectric;
	#ifdef USE_DIFFUSE_ROUGHNESS
		float dotNV = saturate( dot( geometryNormal, geometryViewDir ) );
		vec3 diffuse = EON_DirectionalAlbedo( material.diffuseColor, material.diffuseRoughness, dotNV ) * ( 1.0 - material.metalness ) * ( 1.0 - totalScatteringDielectric );
	#else
		vec3 diffuse = material.diffuseContribution * ( 1.0 - totalScatteringDielectric );
	#endif
	vec3 cosineWeightedIrradiance = irradiance * RECIPROCAL_PI;
	vec3 indirectSpecular = radiance * singleScattering;
	indirectSpecular += multiScattering * cosineWeightedIrradiance;
	vec3 indirectDiffuse = diffuse * cosineWeightedIrradiance;
	#ifdef USE_SHEEN
		float sheenAlbedo = IBLSheenBRDF( geometryNormal, geometryViewDir, material.sheenRoughness );
		float sheenEnergyComp = 1.0 - max3( material.sheenColor ) * sheenAlbedo;
		indirectSpecular *= sheenEnergyComp;
		indirectDiffuse *= sheenEnergyComp;
	#endif
	reflectedLight.indirectSpecular += indirectSpecular;
	reflectedLight.indirectDiffuse += indirectDiffuse;
}
#define RE_Direct				RE_Direct_Physical
#define RE_Direct_RectArea		RE_Direct_RectArea_Physical
#define RE_IndirectDiffuse		RE_IndirectDiffuse_Physical
#define RE_IndirectSpecular		RE_IndirectSpecular_Physical
float computeSpecularOcclusion( const in float dotNV, const in float ambientOcclusion, const in float roughness ) {
	return saturate( pow( dotNV + ambientOcclusion, exp2( - 16.0 * roughness - 1.0 ) ) - 1.0 + ambientOcclusion );
}`,od=`
vec3 geometryPosition = - vViewPosition;
vec3 geometryNormal = normal;
vec3 geometryViewDir = ( isOrthographic ) ? vec3( 0, 0, 1 ) : normalize( vViewPosition );
vec3 geometryClearcoatNormal = vec3( 0.0 );
#ifdef USE_CLEARCOAT
	geometryClearcoatNormal = clearcoatNormal;
#endif
#ifdef USE_IRIDESCENCE
	float dotNVi = saturate( dot( normal, geometryViewDir ) );
	if ( material.iridescenceThickness == 0.0 ) {
		material.iridescence = 0.0;
	} else {
		material.iridescence = saturate( material.iridescence );
	}
	if ( material.iridescence > 0.0 ) {
		vec3 iridescenceFresnelDielectric = evalIridescence( 1.0, material.iridescenceIOR, dotNVi, material.iridescenceThickness, material.specularColor );
		vec3 iridescenceFresnelMetallic = evalIridescence( 1.0, material.iridescenceIOR, dotNVi, material.iridescenceThickness, material.diffuseColor );
		material.iridescenceFresnel = mix( iridescenceFresnelDielectric, iridescenceFresnelMetallic, material.metalness );
		material.iridescenceF0Dielectric = Schlick_to_F0( iridescenceFresnelDielectric, 1.0, dotNVi );
		material.iridescenceF0Metallic = Schlick_to_F0( iridescenceFresnelMetallic, 1.0, dotNVi );
	}
#endif
#ifdef STANDARD
	float dotNVms = saturate( dot( geometryNormal, geometryViewDir ) );
	material.dfg = texture2D( dfgLUT, vec2( material.roughness, dotNVms ) ).rg;
	#if ( NUM_SUN_LIGHTS > 0 || NUM_DIR_LIGHTS > 0 || NUM_POINT_LIGHTS > 0 || NUM_SPOT_LIGHTS > 0 )
		float EssMs = material.dfg.x + material.dfg.y;
		material.multiScatteringCompensation = 1.0 + material.specularColorBlended * ( 1.0 / EssMs - 1.0 );
	#endif
#endif
IncidentLight directLight;
#if ( NUM_POINT_LIGHTS > 0 ) && defined( RE_Direct )
	PointLight pointLight;
	#if defined( USE_SHADOWMAP ) && NUM_POINT_LIGHT_SHADOWS > 0
	PointLightShadow pointLightShadow;
	#endif
	#pragma unroll_loop_start
	for ( int i = 0; i < NUM_POINT_LIGHTS; i ++ ) {
		pointLight = pointLights[ i ];
		getPointLightInfo( pointLight, geometryPosition, directLight );
		#if defined( USE_SHADOWMAP ) && ( UNROLLED_LOOP_INDEX < NUM_POINT_LIGHT_SHADOWS ) && ( defined( SHADOWMAP_TYPE_PCF ) || defined( SHADOWMAP_TYPE_BASIC ) )
		pointLightShadow = pointLightShadows[ i ];
		directLight.color *= ( directLight.visible && receiveShadow ) ? getPointShadow( pointShadowMap[ i ], pointLightShadow.shadowMapSize, pointLightShadow.shadowIntensity, pointLightShadow.shadowBias, pointLightShadow.shadowRadius, vPointShadowCoord[ i ], pointLightShadow.shadowCameraNear, pointLightShadow.shadowCameraFar ) : 1.0;
		#endif
		RE_Direct( directLight, geometryPosition, geometryNormal, geometryViewDir, geometryClearcoatNormal, material, reflectedLight );
	}
	#pragma unroll_loop_end
#endif
#if ( NUM_SPOT_LIGHTS > 0 ) && defined( RE_Direct )
	SpotLight spotLight;
	vec4 spotColor;
	vec3 spotLightCoord;
	bool inSpotLightMap;
	#if defined( USE_SHADOWMAP ) && NUM_SPOT_LIGHT_SHADOWS > 0
	SpotLightShadow spotLightShadow;
	#endif
	#pragma unroll_loop_start
	for ( int i = 0; i < NUM_SPOT_LIGHTS; i ++ ) {
		spotLight = spotLights[ i ];
		getSpotLightInfo( spotLight, geometryPosition, directLight );
		#if ( UNROLLED_LOOP_INDEX < NUM_SPOT_LIGHT_SHADOWS_WITH_MAPS )
		#define SPOT_LIGHT_MAP_INDEX UNROLLED_LOOP_INDEX
		#elif ( UNROLLED_LOOP_INDEX < NUM_SPOT_LIGHT_SHADOWS )
		#define SPOT_LIGHT_MAP_INDEX NUM_SPOT_LIGHT_MAPS
		#else
		#define SPOT_LIGHT_MAP_INDEX ( UNROLLED_LOOP_INDEX - NUM_SPOT_LIGHT_SHADOWS + NUM_SPOT_LIGHT_SHADOWS_WITH_MAPS )
		#endif
		#if ( SPOT_LIGHT_MAP_INDEX < NUM_SPOT_LIGHT_MAPS )
			spotLightCoord = vSpotLightCoord[ i ].xyz / vSpotLightCoord[ i ].w;
			inSpotLightMap = all( lessThan( abs( spotLightCoord * 2. - 1. ), vec3( 1.0 ) ) );
			spotColor = texture2D( spotLightMap[ SPOT_LIGHT_MAP_INDEX ], spotLightCoord.xy );
			directLight.color = inSpotLightMap ? directLight.color * spotColor.rgb : directLight.color;
		#endif
		#undef SPOT_LIGHT_MAP_INDEX
		#if defined( USE_SHADOWMAP ) && ( UNROLLED_LOOP_INDEX < NUM_SPOT_LIGHT_SHADOWS )
		spotLightShadow = spotLightShadows[ i ];
		directLight.color *= ( directLight.visible && receiveShadow ) ? getShadow( spotShadowMap[ i ], spotLightShadow.shadowMapSize, spotLightShadow.shadowIntensity, spotLightShadow.shadowBias, spotLightShadow.shadowRadius, vSpotLightCoord[ i ] ) : 1.0;
		#endif
		RE_Direct( directLight, geometryPosition, geometryNormal, geometryViewDir, geometryClearcoatNormal, material, reflectedLight );
	}
	#pragma unroll_loop_end
#endif
#if ( NUM_SUN_LIGHTS > 0 ) && defined( RE_Direct )
	SunLight sunLight;
	#if defined( USE_SHADOWMAP ) && NUM_SUN_LIGHT_SHADOWS > 0
	SunLightShadow sunLightShadow;
	#endif
	#pragma unroll_loop_start
	for ( int i = 0; i < NUM_SUN_LIGHTS; i ++ ) {
		sunLight = sunLights[ i ];
		getSunLightInfo( sunLight, directLight );
		#if defined( USE_SHADOWMAP ) && ( UNROLLED_LOOP_INDEX < NUM_SUN_LIGHT_SHADOWS )
		sunLightShadow = sunLightShadows[ i ];
		directLight.color *= ( directLight.visible && receiveShadow ) ? getSunShadow( sunShadowMap[ i ], sunLightShadow, UNROLLED_LOOP_INDEX ) : 1.0;
		#endif
		RE_Direct( directLight, geometryPosition, geometryNormal, geometryViewDir, geometryClearcoatNormal, material, reflectedLight );
	}
	#pragma unroll_loop_end
#endif
#if ( NUM_DIR_LIGHTS > 0 ) && defined( RE_Direct )
	DirectionalLight directionalLight;
	#if defined( USE_SHADOWMAP ) && NUM_DIR_LIGHT_SHADOWS > 0
	DirectionalLightShadow directionalLightShadow;
	#endif
	#pragma unroll_loop_start
	for ( int i = 0; i < NUM_DIR_LIGHTS; i ++ ) {
		directionalLight = directionalLights[ i ];
		getDirectionalLightInfo( directionalLight, directLight );
		#if defined( USE_SHADOWMAP ) && ( UNROLLED_LOOP_INDEX < NUM_DIR_LIGHT_SHADOWS )
		directionalLightShadow = directionalLightShadows[ i ];
		directLight.color *= ( directLight.visible && receiveShadow ) ? getShadow( directionalShadowMap[ i ], directionalLightShadow.shadowMapSize, directionalLightShadow.shadowIntensity, directionalLightShadow.shadowBias, directionalLightShadow.shadowRadius, vDirectionalShadowCoord[ i ] ) : 1.0;
		#endif
		RE_Direct( directLight, geometryPosition, geometryNormal, geometryViewDir, geometryClearcoatNormal, material, reflectedLight );
	}
	#pragma unroll_loop_end
#endif
#if ( NUM_RECT_AREA_LIGHTS > 0 ) && defined( RE_Direct_RectArea )
	RectAreaLight rectAreaLight;
	#pragma unroll_loop_start
	for ( int i = 0; i < NUM_RECT_AREA_LIGHTS; i ++ ) {
		rectAreaLight = rectAreaLights[ i ];
		RE_Direct_RectArea( rectAreaLight, geometryPosition, geometryNormal, geometryViewDir, geometryClearcoatNormal, material, reflectedLight );
	}
	#pragma unroll_loop_end
#endif
#if defined( RE_IndirectDiffuse )
	vec3 iblIrradiance = vec3( 0.0 );
	vec3 irradiance = getAmbientLightIrradiance( ambientLightColor );
	#if defined( USE_LIGHT_PROBES )
		irradiance += getLightProbeIrradiance( lightProbe, geometryNormal );
	#endif
	#if ( NUM_HEMI_LIGHTS > 0 )
		#pragma unroll_loop_start
		for ( int i = 0; i < NUM_HEMI_LIGHTS; i ++ ) {
			irradiance += getHemisphereLightIrradiance( hemisphereLights[ i ], geometryNormal );
		}
		#pragma unroll_loop_end
	#endif
	#if NUM_LIGHT_PROBE_GRIDS > 0
		vec3 probeWorldPos = ( ( vec4( geometryPosition, 1.0 ) - viewMatrix[ 3 ] ) * viewMatrix ).xyz;
		vec3 probeWorldNormal = transformNormalByInverseViewMatrix( geometryNormal, viewMatrix );
		int probeGrid = - 1;
		#pragma unroll_loop_start
		for ( int i = 0; i < NUM_LIGHT_PROBE_GRIDS; i ++ ) {
			if ( isInsideLightProbeGrid( probeWorldPos, probeWorldNormal, probesMin[ i ], probesMax[ i ], probesResolution[ i ] ) ) probeGrid = UNROLLED_LOOP_INDEX;
		}
		#pragma unroll_loop_end
		#pragma unroll_loop_start
		for ( int i = 0; i < NUM_LIGHT_PROBE_GRIDS; i ++ ) {
			if ( probeGrid == UNROLLED_LOOP_INDEX ) irradiance += getLightProbeGridIrradiance( probesSH[ i ], probesMin[ i ], probesMax[ i ], probesResolution[ i ], probeWorldPos, probeWorldNormal );
		}
		#pragma unroll_loop_end
	#endif
#endif
#if defined( RE_IndirectSpecular )
	vec3 radiance = vec3( 0.0 );
	vec3 clearcoatRadiance = vec3( 0.0 );
#endif`,ld=`#if defined( RE_IndirectDiffuse )
	#ifdef USE_LIGHTMAP
		vec4 lightMapTexel = texture2D( lightMap, vLightMapUv );
		vec3 lightMapIrradiance = lightMapTexel.rgb * lightMapIntensity;
		irradiance += lightMapIrradiance;
	#endif
	#if defined( USE_ENVMAP ) && defined( ENVMAP_TYPE_PMREM )
		#if defined( STANDARD ) || defined( LAMBERT ) || defined( PHONG )
			iblIrradiance += getIBLIrradiance( geometryNormal );
		#endif
	#endif
#endif
#if defined( USE_ENVMAP ) && defined( RE_IndirectSpecular )
	#ifdef USE_ANISOTROPY
		vec3 iblRadiance = getIBLAnisotropyRadiance( geometryViewDir, geometryNormal, material.roughness, material.anisotropyB, material.anisotropy );
	#else
		vec3 iblRadiance = getIBLRadiance( geometryViewDir, geometryNormal, material.roughness );
	#endif
	#ifdef USE_RETROREFLECTION
		#ifdef USE_ANISOTROPY
			vec3 retroIBLRadiance = getIBLAnisotropyRetroRadiance( geometryViewDir, geometryNormal, material.roughness, material.anisotropyB, material.anisotropy );
		#else
			vec3 retroIBLRadiance = getIBLRetroRadiance( geometryViewDir, geometryNormal, material.roughness );
		#endif
		iblRadiance = mix( iblRadiance, retroIBLRadiance, saturate( material.retroreflectivity ) );
	#endif
	radiance += iblRadiance;
	#ifdef USE_CLEARCOAT
		clearcoatRadiance += getIBLRadiance( geometryViewDir, geometryClearcoatNormal, material.clearcoatRoughness );
	#endif
#endif`,cd=`#if defined( RE_IndirectDiffuse )
	#if defined( LAMBERT ) || defined( PHONG )
		irradiance += iblIrradiance;
	#endif
	RE_IndirectDiffuse( irradiance, geometryPosition, geometryNormal, geometryViewDir, geometryClearcoatNormal, material, reflectedLight );
#endif
#if defined( RE_IndirectSpecular )
	RE_IndirectSpecular( radiance, iblIrradiance, clearcoatRadiance, geometryPosition, geometryNormal, geometryViewDir, geometryClearcoatNormal, material, reflectedLight );
#endif`,hd=`#if NUM_LIGHT_PROBE_GRIDS > 0
uniform highp sampler3D probesSH[ NUM_LIGHT_PROBE_GRIDS ];
uniform vec3 probesMin[ NUM_LIGHT_PROBE_GRIDS ];
uniform vec3 probesMax[ NUM_LIGHT_PROBE_GRIDS ];
uniform vec3 probesResolution[ NUM_LIGHT_PROBE_GRIDS ];
bool isInsideLightProbeGrid( vec3 worldPos, vec3 worldNormal, vec3 probesMin, vec3 probesMax, vec3 probesResolution ) {
	vec3 spacing = ( probesMax - probesMin ) / probesResolution;
	vec3 samplePos = worldPos + worldNormal * spacing * 0.5;
	vec3 margin = spacing * 0.01;
	return all( greaterThanEqual( samplePos, probesMin - margin ) ) && all( lessThanEqual( samplePos, probesMax + margin ) );
}
vec3 getLightProbeGridIrradiance( highp sampler3D probesSH, vec3 probesMin, vec3 probesMax, vec3 probesResolution, vec3 worldPos, vec3 worldNormal ) {
	vec3 res = probesResolution;
	vec3 gridRange = probesMax - probesMin;
	vec3 probeSpacing = gridRange / res;
	vec3 samplePos = worldPos + worldNormal * probeSpacing * 0.5;
	vec3 uvw = clamp( ( samplePos - probesMin ) / gridRange, 0.5 / res, 1.0 - 0.5 / res );
	float nz          = res.z;
	float paddedSlices = nz + 2.0;
	float atlasDepth  = 7.0 * paddedSlices;
	float uvZBase     = uvw.z * nz + 1.0;
	vec4 s0 = texture( probesSH, vec3( uvw.xy, ( uvZBase                       ) / atlasDepth ) );
	vec4 s1 = texture( probesSH, vec3( uvw.xy, ( uvZBase +       paddedSlices   ) / atlasDepth ) );
	vec4 s2 = texture( probesSH, vec3( uvw.xy, ( uvZBase + 2.0 * paddedSlices   ) / atlasDepth ) );
	vec4 s3 = texture( probesSH, vec3( uvw.xy, ( uvZBase + 3.0 * paddedSlices   ) / atlasDepth ) );
	vec4 s4 = texture( probesSH, vec3( uvw.xy, ( uvZBase + 4.0 * paddedSlices   ) / atlasDepth ) );
	vec4 s5 = texture( probesSH, vec3( uvw.xy, ( uvZBase + 5.0 * paddedSlices   ) / atlasDepth ) );
	vec4 s6 = texture( probesSH, vec3( uvw.xy, ( uvZBase + 6.0 * paddedSlices   ) / atlasDepth ) );
	vec3 c0 = s0.xyz;
	vec3 c1 = vec3( s0.w, s1.xy );
	vec3 c2 = vec3( s1.zw, s2.x );
	vec3 c3 = s2.yzw;
	vec3 c4 = s3.xyz;
	vec3 c5 = vec3( s3.w, s4.xy );
	vec3 c6 = vec3( s4.zw, s5.x );
	vec3 c7 = s5.yzw;
	vec3 c8 = s6.xyz;
	float x = worldNormal.x, y = worldNormal.y, z = worldNormal.z;
	vec3 result = c0 * 0.886227;
	result += c1 * 2.0 * 0.511664 * y;
	result += c2 * 2.0 * 0.511664 * z;
	result += c3 * 2.0 * 0.511664 * x;
	result += c4 * 2.0 * 0.429043 * x * y;
	result += c5 * 2.0 * 0.429043 * y * z;
	result += c6 * ( 0.743125 * z * z - 0.247708 );
	result += c7 * 2.0 * 0.429043 * x * z;
	result += c8 * 0.429043 * ( x * x - y * y );
	return max( result, vec3( 0.0 ) );
}
#endif`,ud=`#if defined( USE_LOGARITHMIC_DEPTH_BUFFER )
	gl_FragDepth = vIsPerspective == 0.0 ? gl_FragCoord.z : log2( vFragDepth ) * logDepthBufFC * 0.5;
#endif`,dd=`#if defined( USE_LOGARITHMIC_DEPTH_BUFFER )
	uniform float logDepthBufFC;
	varying float vFragDepth;
	varying float vIsPerspective;
#endif`,fd=`#ifdef USE_LOGARITHMIC_DEPTH_BUFFER
	varying float vFragDepth;
	varying float vIsPerspective;
#endif`,pd=`#ifdef USE_LOGARITHMIC_DEPTH_BUFFER
	vFragDepth = 1.0 + gl_Position.w;
	vIsPerspective = float( isPerspectiveMatrix( projectionMatrix ) );
#endif`,md=`#ifdef USE_MAP
	vec4 sampledDiffuseColor = texture2D( map, vMapUv );
	#ifdef DECODE_VIDEO_TEXTURE
		sampledDiffuseColor = sRGBTransferEOTF( sampledDiffuseColor );
	#endif
	diffuseColor *= sampledDiffuseColor;
#endif`,gd=`#ifdef USE_MAP
	uniform sampler2D map;
#endif`,_d=`#if defined( USE_MAP ) || defined( USE_ALPHAMAP )
	#if defined( USE_POINTS_UV )
		vec2 uv = vUv;
	#else
		vec2 uv = ( uvTransform * vec3( gl_PointCoord.x, 1.0 - gl_PointCoord.y, 1 ) ).xy;
	#endif
#endif
#ifdef USE_MAP
	diffuseColor *= texture2D( map, uv );
#endif
#ifdef USE_ALPHAMAP
	diffuseColor.a *= texture2D( alphaMap, uv ).g;
#endif`,xd=`#if defined( USE_POINTS_UV )
	varying vec2 vUv;
#else
	#if defined( USE_MAP ) || defined( USE_ALPHAMAP )
		uniform mat3 uvTransform;
	#endif
#endif
#ifdef USE_MAP
	uniform sampler2D map;
#endif
#ifdef USE_ALPHAMAP
	uniform sampler2D alphaMap;
#endif`,vd=`float metalnessFactor = metalness;
#ifdef USE_METALNESSMAP
	vec4 texelMetalness = texture2D( metalnessMap, vMetalnessMapUv );
	metalnessFactor *= texelMetalness.b;
#endif`,yd=`#ifdef USE_METALNESSMAP
	uniform sampler2D metalnessMap;
#endif`,Md=`#ifdef USE_INSTANCING_MORPH
	float morphTargetInfluences[ MORPHTARGETS_COUNT ];
	float morphTargetBaseInfluence = texelFetch( morphTexture, ivec2( 0, gl_InstanceID ), 0 ).r;
	for ( int i = 0; i < MORPHTARGETS_COUNT; i ++ ) {
		morphTargetInfluences[i] =  texelFetch( morphTexture, ivec2( i + 1, gl_InstanceID ), 0 ).r;
	}
#endif`,Sd=`#if defined( USE_MORPHCOLORS )
	#if defined( USE_COLOR_ALPHA )
		vColor *= morphTargetBaseInfluence;
	#elif defined( USE_COLOR )
		vColor.rgb *= morphTargetBaseInfluence;
	#endif
	for ( int i = 0; i < MORPHTARGETS_COUNT; i ++ ) {
		#if defined( USE_COLOR_ALPHA )
			if ( morphTargetInfluences[ i ] != 0.0 ) vColor += getMorph( gl_VertexID, i, 2 ) * morphTargetInfluences[ i ];
		#elif defined( USE_COLOR )
			if ( morphTargetInfluences[ i ] != 0.0 ) vColor.rgb += getMorph( gl_VertexID, i, 2 ).rgb * morphTargetInfluences[ i ];
		#endif
	}
#endif`,bd=`#ifdef USE_MORPHNORMALS
	objectNormal *= morphTargetBaseInfluence;
	for ( int i = 0; i < MORPHTARGETS_COUNT; i ++ ) {
		if ( morphTargetInfluences[ i ] != 0.0 ) objectNormal += getMorph( gl_VertexID, i, 1 ).xyz * morphTargetInfluences[ i ];
	}
#endif`,wd=`#ifdef USE_MORPHTARGETS
	#ifndef USE_INSTANCING_MORPH
		uniform float morphTargetBaseInfluence;
		uniform float morphTargetInfluences[ MORPHTARGETS_COUNT ];
	#endif
	uniform sampler2DArray morphTargetsTexture;
	uniform ivec2 morphTargetsTextureSize;
	vec4 getMorph( const in int vertexIndex, const in int morphTargetIndex, const in int offset ) {
		int texelIndex = vertexIndex * MORPHTARGETS_TEXTURE_STRIDE + offset;
		int y = texelIndex / morphTargetsTextureSize.x;
		int x = texelIndex - y * morphTargetsTextureSize.x;
		ivec3 morphUV = ivec3( x, y, morphTargetIndex );
		return texelFetch( morphTargetsTexture, morphUV, 0 );
	}
#endif`,Ad=`#ifdef USE_MORPHTARGETS
	transformed *= morphTargetBaseInfluence;
	for ( int i = 0; i < MORPHTARGETS_COUNT; i ++ ) {
		if ( morphTargetInfluences[ i ] != 0.0 ) transformed += getMorph( gl_VertexID, i, 0 ).xyz * morphTargetInfluences[ i ];
	}
#endif`,Td=`float faceDirection = gl_FrontFacing ? 1.0 : - 1.0;
#ifdef FLAT_SHADED
	vec3 fdx = dFdx( vViewPosition );
	vec3 fdy = dFdy( vViewPosition );
	vec3 normal = normalize( cross( fdx, fdy ) );
#else
	vec3 normal = normalize( vNormal );
	#ifdef DOUBLE_SIDED
		normal *= faceDirection;
	#endif
#endif
#if defined( USE_NORMALMAP_TANGENTSPACE ) || defined( USE_CLEARCOAT_NORMALMAP ) || defined( USE_ANISOTROPY )
	#ifdef USE_TANGENT
		mat3 tbn = mat3( normalize( vTangent ), normalize( vBitangent ), normal );
	#else
		mat3 tbn = getTangentFrame( - vViewPosition, normal,
		#if defined( USE_NORMALMAP )
			vNormalMapUv
		#elif defined( USE_CLEARCOAT_NORMALMAP )
			vClearcoatNormalMapUv
		#else
			vUv
		#endif
		);
	#endif
	#ifdef DOUBLE_SIDED
		tbn[0] *= faceDirection;
		tbn[1] *= faceDirection;
	#endif
#endif
#ifdef USE_CLEARCOAT_NORMALMAP
	#ifdef USE_TANGENT
		mat3 tbn2 = mat3( normalize( vTangent ), normalize( vBitangent ), normal );
	#else
		mat3 tbn2 = getTangentFrame( - vViewPosition, normal, vClearcoatNormalMapUv );
	#endif
	#ifdef DOUBLE_SIDED
		tbn2[0] *= faceDirection;
		tbn2[1] *= faceDirection;
	#endif
#endif
vec3 nonPerturbedNormal = normal;`,Ed=`#ifdef USE_NORMALMAP_OBJECTSPACE
	normal = texture2D( normalMap, vNormalMapUv ).xyz * 2.0 - 1.0;
	#ifdef FLIP_SIDED
		normal = - normal;
	#endif
	#ifdef DOUBLE_SIDED
		normal = normal * faceDirection;
	#endif
	normal = normalize( normalMatrix * normal );
#elif defined( USE_NORMALMAP_TANGENTSPACE )
	vec3 mapN = texture2D( normalMap, vNormalMapUv ).xyz * 2.0 - 1.0;
	#if defined( USE_PACKED_NORMALMAP )
		mapN = vec3( mapN.xy, sqrt( saturate( 1.0 - dot( mapN.xy, mapN.xy ) ) ) );
	#endif
	mapN.xy *= normalScale;
	normal = normalize( tbn * mapN );
#elif defined( USE_BUMPMAP )
	normal = perturbNormalArb( - vViewPosition, normal, dHdxy_fwd(), faceDirection );
#endif`,Rd=`#ifndef FLAT_SHADED
	varying vec3 vNormal;
	#ifdef USE_TANGENT
		varying vec3 vTangent;
		varying vec3 vBitangent;
	#endif
#endif`,Cd=`#ifndef FLAT_SHADED
	varying vec3 vNormal;
	#ifdef USE_TANGENT
		varying vec3 vTangent;
		varying vec3 vBitangent;
	#endif
#endif`,Id=`#ifndef FLAT_SHADED
	vNormal = normalize( transformedNormal );
	#ifdef USE_TANGENT
		vTangent = normalize( transformedTangent );
		vBitangent = normalize( cross( vNormal, vTangent ) * tangent.w );
		#ifdef FLIP_SIDED
			vBitangent = - vBitangent;
		#endif
	#endif
#endif`,Pd=`#ifdef USE_NORMALMAP
	uniform sampler2D normalMap;
	uniform vec2 normalScale;
#endif
#ifdef USE_NORMALMAP_OBJECTSPACE
	uniform mat3 normalMatrix;
#endif
#if ! defined ( USE_TANGENT ) && ( defined ( USE_NORMALMAP_TANGENTSPACE ) || defined ( USE_CLEARCOAT_NORMALMAP ) || defined( USE_ANISOTROPY ) )
	mat3 getTangentFrame( vec3 eye_pos, vec3 surf_norm, vec2 uv ) {
		vec3 q0 = dFdx( eye_pos.xyz );
		vec3 q1 = dFdy( eye_pos.xyz );
		vec2 st0 = dFdx( uv.st );
		vec2 st1 = dFdy( uv.st );
		vec3 N = surf_norm;
		vec3 q1perp = cross( q1, N );
		vec3 q0perp = cross( N, q0 );
		vec3 T = q1perp * st0.x + q0perp * st1.x;
		vec3 B = q1perp * st0.y + q0perp * st1.y;
		float det = max( dot( T, T ), dot( B, B ) );
		float scale = ( det == 0.0 ) ? 0.0 : inversesqrt( det );
		return mat3( T * scale, B * scale, N );
	}
#endif`,Ld=`#ifdef USE_CLEARCOAT
	vec3 clearcoatNormal = nonPerturbedNormal;
#endif`,Nd=`#ifdef USE_CLEARCOAT_NORMALMAP
	vec3 clearcoatMapN = texture2D( clearcoatNormalMap, vClearcoatNormalMapUv ).xyz * 2.0 - 1.0;
	clearcoatMapN.xy *= clearcoatNormalScale;
	clearcoatNormal = normalize( tbn2 * clearcoatMapN );
#endif`,Dd=`#ifdef USE_CLEARCOATMAP
	uniform sampler2D clearcoatMap;
#endif
#ifdef USE_CLEARCOAT_NORMALMAP
	uniform sampler2D clearcoatNormalMap;
	uniform vec2 clearcoatNormalScale;
#endif
#ifdef USE_CLEARCOAT_ROUGHNESSMAP
	uniform sampler2D clearcoatRoughnessMap;
#endif`,Ud=`#ifdef USE_IRIDESCENCEMAP
	uniform sampler2D iridescenceMap;
#endif
#ifdef USE_IRIDESCENCE_THICKNESSMAP
	uniform sampler2D iridescenceThicknessMap;
#endif`,Fd=`#ifdef OPAQUE
diffuseColor.a = 1.0;
#endif
#ifdef USE_TRANSMISSION
diffuseColor.a *= material.transmissionAlpha;
#endif
gl_FragColor = vec4( outgoingLight, diffuseColor.a );`,Od=`vec3 packNormalToRGB( const in vec3 normal ) {
	return normalize( normal ) * 0.5 + 0.5;
}
vec3 unpackRGBToNormal( const in vec3 rgb ) {
	return 2.0 * rgb.xyz - 1.0;
}
const float PackUpscale = 256. / 255.;const float UnpackDownscale = 255. / 256.;const float ShiftRight8 = 1. / 256.;
const float Inv255 = 1. / 255.;
const vec4 PackFactors = vec4( 1.0, 256.0, 256.0 * 256.0, 256.0 * 256.0 * 256.0 );
const vec2 UnpackFactors2 = vec2( UnpackDownscale, 1.0 / PackFactors.g );
const vec3 UnpackFactors3 = vec3( UnpackDownscale / PackFactors.rg, 1.0 / PackFactors.b );
const vec4 UnpackFactors4 = vec4( UnpackDownscale / PackFactors.rgb, 1.0 / PackFactors.a );
vec4 packDepthToRGBA( const in float v ) {
	if( v <= 0.0 )
		return vec4( 0., 0., 0., 0. );
	if( v >= 1.0 )
		return vec4( 1., 1., 1., 1. );
	float vuf;
	float af = modf( v * PackFactors.a, vuf );
	float bf = modf( vuf * ShiftRight8, vuf );
	float gf = modf( vuf * ShiftRight8, vuf );
	return vec4( vuf * Inv255, gf * PackUpscale, bf * PackUpscale, af );
}
vec3 packDepthToRGB( const in float v ) {
	if( v <= 0.0 )
		return vec3( 0., 0., 0. );
	if( v >= 1.0 )
		return vec3( 1., 1., 1. );
	float vuf;
	float bf = modf( v * PackFactors.b, vuf );
	float gf = modf( vuf * ShiftRight8, vuf );
	return vec3( vuf * Inv255, gf * PackUpscale, bf );
}
vec2 packDepthToRG( const in float v ) {
	if( v <= 0.0 )
		return vec2( 0., 0. );
	if( v >= 1.0 )
		return vec2( 1., 1. );
	float vuf;
	float gf = modf( v * 256., vuf );
	return vec2( vuf * Inv255, gf );
}
float unpackRGBAToDepth( const in vec4 v ) {
	return dot( v, UnpackFactors4 );
}
float unpackRGBToDepth( const in vec3 v ) {
	return dot( v, UnpackFactors3 );
}
float unpackRGToDepth( const in vec2 v ) {
	return v.r * UnpackFactors2.r + v.g * UnpackFactors2.g;
}
vec4 pack2HalfToRGBA( const in vec2 v ) {
	vec4 r = vec4( v.x, fract( v.x * 255.0 ), v.y, fract( v.y * 255.0 ) );
	return vec4( r.x - r.y / 255.0, r.y, r.z - r.w / 255.0, r.w );
}
vec2 unpackRGBATo2Half( const in vec4 v ) {
	return vec2( v.x + ( v.y / 255.0 ), v.z + ( v.w / 255.0 ) );
}
float viewZToOrthographicDepth( const in float viewZ, const in float near, const in float far ) {
	return ( viewZ + near ) / ( near - far );
}
float orthographicDepthToViewZ( const in float depth, const in float near, const in float far ) {
	#ifdef USE_REVERSED_DEPTH_BUFFER
	
		return depth * ( far - near ) - far;
	#else
		return depth * ( near - far ) - near;
	#endif
}
float viewZToPerspectiveDepth( const in float viewZ, const in float near, const in float far ) {
	return ( ( near + viewZ ) * far ) / ( ( far - near ) * viewZ );
}
float perspectiveDepthToViewZ( const in float depth, const in float near, const in float far ) {
	
	#ifdef USE_REVERSED_DEPTH_BUFFER
		return ( near * far ) / ( ( near - far ) * depth - near );
	#else
		return ( near * far ) / ( ( far - near ) * depth - far );
	#endif
}`,Bd=`#ifdef PREMULTIPLIED_ALPHA
	gl_FragColor.rgb *= gl_FragColor.a;
#endif`,zd=`vec4 mvPosition = vec4( transformed, 1.0 );
#ifdef USE_BATCHING
	mvPosition = batchingMatrix * mvPosition;
#endif
#ifdef USE_INSTANCING
	mvPosition = instanceMatrix * mvPosition;
#endif
mvPosition = modelViewMatrix * mvPosition;
gl_Position = projectionMatrix * mvPosition;`,kd=`#ifdef DITHERING
	gl_FragColor.rgb = dithering( gl_FragColor.rgb );
#endif`,Gd=`#ifdef DITHERING
	vec3 dithering( vec3 color ) {
		float grid_position = rand( gl_FragCoord.xy );
		vec3 dither_shift_RGB = vec3( 0.25 / 255.0, -0.25 / 255.0, 0.25 / 255.0 );
		dither_shift_RGB = mix( 2.0 * dither_shift_RGB, -2.0 * dither_shift_RGB, grid_position );
		return color + dither_shift_RGB;
	}
#endif`,Hd=`float roughnessFactor = roughness;
#ifdef USE_ROUGHNESSMAP
	vec4 texelRoughness = texture2D( roughnessMap, vRoughnessMapUv );
	roughnessFactor *= texelRoughness.g;
#endif`,Wd=`#ifdef USE_ROUGHNESSMAP
	uniform sampler2D roughnessMap;
#endif`,Vd=`#if NUM_SPOT_LIGHT_COORDS > 0
	varying vec4 vSpotLightCoord[ NUM_SPOT_LIGHT_COORDS ];
#endif
#if NUM_SPOT_LIGHT_MAPS > 0
	uniform sampler2D spotLightMap[ NUM_SPOT_LIGHT_MAPS ];
#endif
#ifdef USE_SHADOWMAP
	#if NUM_SUN_LIGHT_SHADOWS > 0
		#define SUN_LIGHT_CASCADES 2
		#if defined( SHADOWMAP_TYPE_PCF )
			uniform sampler2DShadow sunShadowMap[ NUM_SUN_LIGHT_SHADOWS ];
		#else
			uniform sampler2D sunShadowMap[ NUM_SUN_LIGHT_SHADOWS ];
		#endif
		uniform mat4 sunShadowMatrix[ NUM_SUN_LIGHT_SHADOWS * SUN_LIGHT_CASCADES ];
		uniform vec4 sunShadowCascade[ NUM_SUN_LIGHT_SHADOWS * SUN_LIGHT_CASCADES ];
		varying vec4 vSunShadowWorldPosition;
		varying vec3 vSunShadowWorldNormal;
		struct SunLightShadow {
			float shadowIntensity;
			float shadowBias;
			float shadowNormalBias;
			float shadowRadius;
			vec2 shadowMapSize;
		};
		uniform SunLightShadow sunLightShadows[ NUM_SUN_LIGHT_SHADOWS ];
	#endif
	#if NUM_DIR_LIGHT_SHADOWS > 0
		#if defined( SHADOWMAP_TYPE_PCF )
			uniform sampler2DShadow directionalShadowMap[ NUM_DIR_LIGHT_SHADOWS ];
		#else
			uniform sampler2D directionalShadowMap[ NUM_DIR_LIGHT_SHADOWS ];
		#endif
		varying vec4 vDirectionalShadowCoord[ NUM_DIR_LIGHT_SHADOWS ];
		struct DirectionalLightShadow {
			float shadowIntensity;
			float shadowBias;
			float shadowNormalBias;
			float shadowRadius;
			vec2 shadowMapSize;
		};
		uniform DirectionalLightShadow directionalLightShadows[ NUM_DIR_LIGHT_SHADOWS ];
	#endif
	#if NUM_SPOT_LIGHT_SHADOWS > 0
		#if defined( SHADOWMAP_TYPE_PCF )
			uniform sampler2DShadow spotShadowMap[ NUM_SPOT_LIGHT_SHADOWS ];
		#else
			uniform sampler2D spotShadowMap[ NUM_SPOT_LIGHT_SHADOWS ];
		#endif
		struct SpotLightShadow {
			float shadowIntensity;
			float shadowBias;
			float shadowNormalBias;
			float shadowRadius;
			vec2 shadowMapSize;
		};
		uniform SpotLightShadow spotLightShadows[ NUM_SPOT_LIGHT_SHADOWS ];
	#endif
	#if NUM_POINT_LIGHT_SHADOWS > 0
		#if defined( SHADOWMAP_TYPE_PCF )
			uniform samplerCubeShadow pointShadowMap[ NUM_POINT_LIGHT_SHADOWS ];
		#elif defined( SHADOWMAP_TYPE_BASIC )
			uniform samplerCube pointShadowMap[ NUM_POINT_LIGHT_SHADOWS ];
		#endif
		varying vec4 vPointShadowCoord[ NUM_POINT_LIGHT_SHADOWS ];
		struct PointLightShadow {
			float shadowIntensity;
			float shadowBias;
			float shadowNormalBias;
			float shadowRadius;
			vec2 shadowMapSize;
			float shadowCameraNear;
			float shadowCameraFar;
		};
		uniform PointLightShadow pointLightShadows[ NUM_POINT_LIGHT_SHADOWS ];
	#endif
	#if defined( SHADOWMAP_TYPE_PCF )
		float interleavedGradientNoise( vec2 position ) {
			return fract( 52.9829189 * fract( dot( position, vec2( 0.06711056, 0.00583715 ) ) ) );
		}
		vec2 vogelDiskSample( int sampleIndex, int samplesCount, float phi ) {
			const float goldenAngle = 2.399963229728653;
			float r = sqrt( ( float( sampleIndex ) + 0.5 ) / float( samplesCount ) );
			float theta = float( sampleIndex ) * goldenAngle + phi;
			return vec2( cos( theta ), sin( theta ) ) * r;
		}
	#endif
	#if defined( SHADOWMAP_TYPE_PCF )
		float getShadow( sampler2DShadow shadowMap, vec2 shadowMapSize, float shadowIntensity, float shadowBias, float shadowRadius, vec4 shadowCoord ) {
			float shadow = 1.0;
			shadowCoord.xyz /= shadowCoord.w;
			#ifdef USE_REVERSED_DEPTH_BUFFER
				shadowCoord.z -= shadowBias;
			#else
				shadowCoord.z += shadowBias;
			#endif
			bool inFrustum = shadowCoord.x >= 0.0 && shadowCoord.x <= 1.0 && shadowCoord.y >= 0.0 && shadowCoord.y <= 1.0;
			bool frustumTest = inFrustum && shadowCoord.z <= 1.0;
			if ( frustumTest ) {
				vec2 texelSize = vec2( 1.0 ) / shadowMapSize;
				vec2 radius = shadowRadius * texelSize;
				float phi = interleavedGradientNoise( gl_FragCoord.xy ) * PI2;
				shadow = (
					texture( shadowMap, vec3( shadowCoord.xy + vogelDiskSample( 0, 5, phi ) * radius, shadowCoord.z ) ) +
					texture( shadowMap, vec3( shadowCoord.xy + vogelDiskSample( 1, 5, phi ) * radius, shadowCoord.z ) ) +
					texture( shadowMap, vec3( shadowCoord.xy + vogelDiskSample( 2, 5, phi ) * radius, shadowCoord.z ) ) +
					texture( shadowMap, vec3( shadowCoord.xy + vogelDiskSample( 3, 5, phi ) * radius, shadowCoord.z ) ) +
					texture( shadowMap, vec3( shadowCoord.xy + vogelDiskSample( 4, 5, phi ) * radius, shadowCoord.z ) )
				) * 0.2;
			}
			return mix( 1.0, shadow, shadowIntensity );
		}
	#elif defined( SHADOWMAP_TYPE_VSM )
		float getShadow( sampler2D shadowMap, vec2 shadowMapSize, float shadowIntensity, float shadowBias, float shadowRadius, vec4 shadowCoord ) {
			float shadow = 1.0;
			shadowCoord.xyz /= shadowCoord.w;
			#ifdef USE_REVERSED_DEPTH_BUFFER
				shadowCoord.z -= shadowBias;
			#else
				shadowCoord.z += shadowBias;
			#endif
			bool inFrustum = shadowCoord.x >= 0.0 && shadowCoord.x <= 1.0 && shadowCoord.y >= 0.0 && shadowCoord.y <= 1.0;
			bool frustumTest = inFrustum && shadowCoord.z <= 1.0;
			if ( frustumTest ) {
				vec2 distribution = texture2D( shadowMap, shadowCoord.xy ).rg;
				float mean = distribution.x;
				float variance = distribution.y * distribution.y;
				#ifdef USE_REVERSED_DEPTH_BUFFER
					float hard_shadow = step( mean, shadowCoord.z );
				#else
					float hard_shadow = step( shadowCoord.z, mean );
				#endif
				
				if ( hard_shadow == 1.0 ) {
					shadow = 1.0;
				} else {
					variance = max( variance, 0.0000001 );
					float d = shadowCoord.z - mean;
					float p_max = variance / ( variance + d * d );
					p_max = clamp( ( p_max - 0.3 ) / 0.65, 0.0, 1.0 );
					shadow = max( hard_shadow, p_max );
				}
			}
			return mix( 1.0, shadow, shadowIntensity );
		}
	#else
		float getShadow( sampler2D shadowMap, vec2 shadowMapSize, float shadowIntensity, float shadowBias, float shadowRadius, vec4 shadowCoord ) {
			float shadow = 1.0;
			shadowCoord.xyz /= shadowCoord.w;
			#ifdef USE_REVERSED_DEPTH_BUFFER
				shadowCoord.z -= shadowBias;
			#else
				shadowCoord.z += shadowBias;
			#endif
			bool inFrustum = shadowCoord.x >= 0.0 && shadowCoord.x <= 1.0 && shadowCoord.y >= 0.0 && shadowCoord.y <= 1.0;
			bool frustumTest = inFrustum && shadowCoord.z <= 1.0;
			if ( frustumTest ) {
				float depth = texture2D( shadowMap, shadowCoord.xy ).r;
				#ifdef USE_REVERSED_DEPTH_BUFFER
					shadow = step( depth, shadowCoord.z );
				#else
					shadow = step( shadowCoord.z, depth );
				#endif
			}
			return mix( 1.0, shadow, shadowIntensity );
		}
	#endif
	#if NUM_SUN_LIGHT_SHADOWS > 0
		float getSunShadow(
			#if defined( SHADOWMAP_TYPE_PCF )
				sampler2DShadow shadowMap,
			#else
				sampler2D shadowMap,
			#endif
			SunLightShadow sunLightShadow,
			int shadowIndex
		) {
			vec4 shadowWorldPosition = vec4( vSunShadowWorldPosition.xyz + vSunShadowWorldNormal * sunLightShadow.shadowNormalBias, 1.0 );
			float viewDepth = vSunShadowWorldPosition.w;
			int cascadeOffset = shadowIndex * SUN_LIGHT_CASCADES;
			float shadow = 1.0;
			for ( int i = SUN_LIGHT_CASCADES - 1; i >= 0; i -- ) {
				vec4 cascade = sunShadowCascade[ cascadeOffset + i ];
				if ( viewDepth >= cascade.x && viewDepth < cascade.y ) {
					float cascadeShadow = getShadow(
						shadowMap,
						sunLightShadow.shadowMapSize,
						sunLightShadow.shadowIntensity,
						sunLightShadow.shadowBias,
						sunLightShadow.shadowRadius,
						sunShadowMatrix[ cascadeOffset + i ] * shadowWorldPosition
					);
					shadow = mix( cascadeShadow, shadow, smoothstep( cascade.z, cascade.y, viewDepth ) );
				}
			}
			return shadow;
		}
	#endif
	#if NUM_POINT_LIGHT_SHADOWS > 0
	#if defined( SHADOWMAP_TYPE_PCF )
	float getPointShadow( samplerCubeShadow shadowMap, vec2 shadowMapSize, float shadowIntensity, float shadowBias, float shadowRadius, vec4 shadowCoord, float shadowCameraNear, float shadowCameraFar ) {
		float shadow = 1.0;
		vec3 lightToPosition = shadowCoord.xyz;
		vec3 bd3D = normalize( lightToPosition );
		vec3 absVec = abs( lightToPosition );
		float viewSpaceZ = max( max( absVec.x, absVec.y ), absVec.z );
		if ( viewSpaceZ - shadowCameraFar <= 0.0 && viewSpaceZ - shadowCameraNear >= 0.0 ) {
			float lightDistance = length( lightToPosition );
			float depthRange = shadowCameraFar - shadowCameraNear;
			#ifdef USE_REVERSED_DEPTH_BUFFER
				float depthA = - shadowCameraNear / depthRange - shadowBias;
				float depthB = shadowCameraNear * shadowCameraFar / ( depthRange * lightDistance );
			#else
				float depthA = shadowCameraFar / depthRange + shadowBias;
				float depthB = - shadowCameraNear * shadowCameraFar / ( depthRange * lightDistance );
			#endif
			float texelSize = shadowRadius / shadowMapSize.x;
			vec3 absDir = abs( bd3D );
			vec3 tangent = absDir.x > absDir.z ? vec3( 0.0, 1.0, 0.0 ) : vec3( 1.0, 0.0, 0.0 );
			tangent = normalize( cross( bd3D, tangent ) );
			vec3 bitangent = cross( bd3D, tangent );
			float phi = interleavedGradientNoise( gl_FragCoord.xy ) * PI2;
			shadow = 0.0;
			for ( int i = 0; i < 5; i ++ ) {
				vec2 offset = vogelDiskSample( i, 5, phi );
				vec3 direction = bd3D + ( tangent * offset.x + bitangent * offset.y ) * texelSize;
				vec3 absDirection = abs( direction );
				float dp = depthA + depthB / max( max( absDirection.x, absDirection.y ), absDirection.z );
				shadow += texture( shadowMap, vec4( direction, dp ) );
			}
			shadow *= 0.2;
		}
		return mix( 1.0, shadow, shadowIntensity );
	}
	#elif defined( SHADOWMAP_TYPE_BASIC )
	float getPointShadow( samplerCube shadowMap, vec2 shadowMapSize, float shadowIntensity, float shadowBias, float shadowRadius, vec4 shadowCoord, float shadowCameraNear, float shadowCameraFar ) {
		float shadow = 1.0;
		vec3 lightToPosition = shadowCoord.xyz;
		vec3 absVec = abs( lightToPosition );
		float viewSpaceZ = max( max( absVec.x, absVec.y ), absVec.z );
		if ( viewSpaceZ - shadowCameraFar <= 0.0 && viewSpaceZ - shadowCameraNear >= 0.0 ) {
			float dp = ( shadowCameraFar * ( viewSpaceZ - shadowCameraNear ) ) / ( viewSpaceZ * ( shadowCameraFar - shadowCameraNear ) );
			dp += shadowBias;
			vec3 bd3D = normalize( lightToPosition );
			float depth = textureCube( shadowMap, bd3D ).r;
			#ifdef USE_REVERSED_DEPTH_BUFFER
				depth = 1.0 - depth;
			#endif
			shadow = step( dp, depth );
		}
		return mix( 1.0, shadow, shadowIntensity );
	}
	#endif
	#endif
#endif`,Xd=`#if NUM_SPOT_LIGHT_COORDS > 0
	uniform mat4 spotLightMatrix[ NUM_SPOT_LIGHT_COORDS ];
	varying vec4 vSpotLightCoord[ NUM_SPOT_LIGHT_COORDS ];
#endif
#ifdef USE_SHADOWMAP
	#if NUM_SUN_LIGHT_SHADOWS > 0
		varying vec4 vSunShadowWorldPosition;
		varying vec3 vSunShadowWorldNormal;
	#endif
	#if NUM_DIR_LIGHT_SHADOWS > 0
		uniform mat4 directionalShadowMatrix[ NUM_DIR_LIGHT_SHADOWS ];
		varying vec4 vDirectionalShadowCoord[ NUM_DIR_LIGHT_SHADOWS ];
		struct DirectionalLightShadow {
			float shadowIntensity;
			float shadowBias;
			float shadowNormalBias;
			float shadowRadius;
			vec2 shadowMapSize;
		};
		uniform DirectionalLightShadow directionalLightShadows[ NUM_DIR_LIGHT_SHADOWS ];
	#endif
	#if NUM_SPOT_LIGHT_SHADOWS > 0
		struct SpotLightShadow {
			float shadowIntensity;
			float shadowBias;
			float shadowNormalBias;
			float shadowRadius;
			vec2 shadowMapSize;
		};
		uniform SpotLightShadow spotLightShadows[ NUM_SPOT_LIGHT_SHADOWS ];
	#endif
	#if NUM_POINT_LIGHT_SHADOWS > 0
		uniform mat4 pointShadowMatrix[ NUM_POINT_LIGHT_SHADOWS ];
		varying vec4 vPointShadowCoord[ NUM_POINT_LIGHT_SHADOWS ];
		struct PointLightShadow {
			float shadowIntensity;
			float shadowBias;
			float shadowNormalBias;
			float shadowRadius;
			vec2 shadowMapSize;
			float shadowCameraNear;
			float shadowCameraFar;
		};
		uniform PointLightShadow pointLightShadows[ NUM_POINT_LIGHT_SHADOWS ];
	#endif
#endif`,qd=`#if ( defined( USE_SHADOWMAP ) && ( NUM_DIR_LIGHT_SHADOWS > 0 || NUM_SUN_LIGHT_SHADOWS > 0 || NUM_POINT_LIGHT_SHADOWS > 0 ) ) || ( NUM_SPOT_LIGHT_COORDS > 0 )
	#ifdef HAS_NORMAL
		vec3 shadowWorldNormal = transformNormalByInverseViewMatrix( transformedNormal, viewMatrix );
	#else
		vec3 shadowWorldNormal = vec3( 0.0 );
	#endif
	vec4 shadowWorldPosition;
#endif
#if defined( USE_SHADOWMAP )
	#if NUM_SUN_LIGHT_SHADOWS > 0
		vSunShadowWorldPosition = vec4( worldPosition.xyz, - mvPosition.z );
		vSunShadowWorldNormal = shadowWorldNormal;
	#endif
	#if NUM_DIR_LIGHT_SHADOWS > 0
		#pragma unroll_loop_start
		for ( int i = 0; i < NUM_DIR_LIGHT_SHADOWS; i ++ ) {
			shadowWorldPosition = worldPosition + vec4( shadowWorldNormal * directionalLightShadows[ i ].shadowNormalBias, 0 );
			vDirectionalShadowCoord[ i ] = directionalShadowMatrix[ i ] * shadowWorldPosition;
		}
		#pragma unroll_loop_end
	#endif
	#if NUM_POINT_LIGHT_SHADOWS > 0
		#pragma unroll_loop_start
		for ( int i = 0; i < NUM_POINT_LIGHT_SHADOWS; i ++ ) {
			shadowWorldPosition = worldPosition + vec4( shadowWorldNormal * pointLightShadows[ i ].shadowNormalBias, 0 );
			vPointShadowCoord[ i ] = pointShadowMatrix[ i ] * shadowWorldPosition;
		}
		#pragma unroll_loop_end
	#endif
#endif
#if NUM_SPOT_LIGHT_COORDS > 0
	#pragma unroll_loop_start
	for ( int i = 0; i < NUM_SPOT_LIGHT_COORDS; i ++ ) {
		shadowWorldPosition = worldPosition;
		#if ( defined( USE_SHADOWMAP ) && UNROLLED_LOOP_INDEX < NUM_SPOT_LIGHT_SHADOWS )
			shadowWorldPosition.xyz += shadowWorldNormal * spotLightShadows[ i ].shadowNormalBias;
		#endif
		vSpotLightCoord[ i ] = spotLightMatrix[ i ] * shadowWorldPosition;
	}
	#pragma unroll_loop_end
#endif`,Yd=`float getShadowMask() {
	float shadow = 1.0;
	#ifdef USE_SHADOWMAP
	#if NUM_SUN_LIGHT_SHADOWS > 0
	SunLightShadow sunLight;
	#pragma unroll_loop_start
	for ( int i = 0; i < NUM_SUN_LIGHT_SHADOWS; i ++ ) {
		sunLight = sunLightShadows[ i ];
		shadow *= receiveShadow ? getSunShadow( sunShadowMap[ i ], sunLight, UNROLLED_LOOP_INDEX ) : 1.0;
	}
	#pragma unroll_loop_end
	#endif
	#if NUM_DIR_LIGHT_SHADOWS > 0
	DirectionalLightShadow directionalLight;
	#pragma unroll_loop_start
	for ( int i = 0; i < NUM_DIR_LIGHT_SHADOWS; i ++ ) {
		directionalLight = directionalLightShadows[ i ];
		shadow *= receiveShadow ? getShadow( directionalShadowMap[ i ], directionalLight.shadowMapSize, directionalLight.shadowIntensity, directionalLight.shadowBias, directionalLight.shadowRadius, vDirectionalShadowCoord[ i ] ) : 1.0;
	}
	#pragma unroll_loop_end
	#endif
	#if NUM_SPOT_LIGHT_SHADOWS > 0
	SpotLightShadow spotLight;
	#pragma unroll_loop_start
	for ( int i = 0; i < NUM_SPOT_LIGHT_SHADOWS; i ++ ) {
		spotLight = spotLightShadows[ i ];
		shadow *= receiveShadow ? getShadow( spotShadowMap[ i ], spotLight.shadowMapSize, spotLight.shadowIntensity, spotLight.shadowBias, spotLight.shadowRadius, vSpotLightCoord[ i ] ) : 1.0;
	}
	#pragma unroll_loop_end
	#endif
	#if NUM_POINT_LIGHT_SHADOWS > 0 && ( defined( SHADOWMAP_TYPE_PCF ) || defined( SHADOWMAP_TYPE_BASIC ) )
	PointLightShadow pointLight;
	#pragma unroll_loop_start
	for ( int i = 0; i < NUM_POINT_LIGHT_SHADOWS; i ++ ) {
		pointLight = pointLightShadows[ i ];
		shadow *= receiveShadow ? getPointShadow( pointShadowMap[ i ], pointLight.shadowMapSize, pointLight.shadowIntensity, pointLight.shadowBias, pointLight.shadowRadius, vPointShadowCoord[ i ], pointLight.shadowCameraNear, pointLight.shadowCameraFar ) : 1.0;
	}
	#pragma unroll_loop_end
	#endif
	#endif
	return shadow;
}`,Zd=`#ifdef USE_SKINNING
	mat4 boneMatX = getBoneMatrix( skinIndex.x );
	mat4 boneMatY = getBoneMatrix( skinIndex.y );
	mat4 boneMatZ = getBoneMatrix( skinIndex.z );
	mat4 boneMatW = getBoneMatrix( skinIndex.w );
#endif`,Jd=`#ifdef USE_SKINNING
	uniform mat4 bindMatrix;
	uniform mat4 bindMatrixInverse;
	uniform highp sampler2D boneTexture;
	mat4 getBoneMatrix( const in float i ) {
		int size = textureSize( boneTexture, 0 ).x;
		int j = int( i ) * 4;
		int x = j % size;
		int y = j / size;
		vec4 v1 = texelFetch( boneTexture, ivec2( x, y ), 0 );
		vec4 v2 = texelFetch( boneTexture, ivec2( x + 1, y ), 0 );
		vec4 v3 = texelFetch( boneTexture, ivec2( x + 2, y ), 0 );
		vec4 v4 = texelFetch( boneTexture, ivec2( x + 3, y ), 0 );
		return mat4( v1, v2, v3, v4 );
	}
#endif`,$d=`#ifdef USE_SKINNING
	vec4 skinVertex = bindMatrix * vec4( transformed, 1.0 );
	vec4 skinned = vec4( 0.0 );
	skinned += boneMatX * skinVertex * skinWeight.x;
	skinned += boneMatY * skinVertex * skinWeight.y;
	skinned += boneMatZ * skinVertex * skinWeight.z;
	skinned += boneMatW * skinVertex * skinWeight.w;
	transformed = ( bindMatrixInverse * skinned ).xyz;
#endif`,Kd=`#ifdef USE_SKINNING
	mat4 skinMatrix = mat4( 0.0 );
	skinMatrix += skinWeight.x * boneMatX;
	skinMatrix += skinWeight.y * boneMatY;
	skinMatrix += skinWeight.z * boneMatZ;
	skinMatrix += skinWeight.w * boneMatW;
	skinMatrix = bindMatrixInverse * skinMatrix * bindMatrix;
	objectNormal = vec4( skinMatrix * vec4( objectNormal, 0.0 ) ).xyz;
	#ifdef USE_TANGENT
		objectTangent = vec4( skinMatrix * vec4( objectTangent, 0.0 ) ).xyz;
	#endif
#endif`,Qd=`float specularStrength;
#ifdef USE_SPECULARMAP
	vec4 texelSpecular = texture2D( specularMap, vSpecularMapUv );
	specularStrength = texelSpecular.r;
#else
	specularStrength = 1.0;
#endif`,jd=`#ifdef USE_SPECULARMAP
	uniform sampler2D specularMap;
#endif`,tf=`#if defined( TONE_MAPPING )
	gl_FragColor.rgb = toneMapping( gl_FragColor.rgb );
#endif`,ef=`#ifndef saturate
#define saturate( a ) clamp( a, 0.0, 1.0 )
#endif
uniform float toneMappingExposure;
vec3 LinearToneMapping( vec3 color ) {
	return saturate( toneMappingExposure * color );
}
vec3 ReinhardToneMapping( vec3 color ) {
	color *= toneMappingExposure;
	return saturate( color / ( vec3( 1.0 ) + color ) );
}
vec3 CineonToneMapping( vec3 color ) {
	color *= toneMappingExposure;
	color = max( vec3( 0.0 ), color - 0.004 );
	return pow( ( color * ( 6.2 * color + 0.5 ) ) / ( color * ( 6.2 * color + 1.7 ) + 0.06 ), vec3( 2.2 ) );
}
vec3 RRTAndODTFit( vec3 v ) {
	vec3 a = v * ( v + 0.0245786 ) - 0.000090537;
	vec3 b = v * ( 0.983729 * v + 0.4329510 ) + 0.238081;
	return a / b;
}
vec3 ACESFilmicToneMapping( vec3 color ) {
	const mat3 ACESInputMat = mat3(
		vec3( 0.59719, 0.07600, 0.02840 ),		vec3( 0.35458, 0.90834, 0.13383 ),
		vec3( 0.04823, 0.01566, 0.83777 )
	);
	const mat3 ACESOutputMat = mat3(
		vec3(  1.60475, -0.10208, -0.00327 ),		vec3( -0.53108,  1.10813, -0.07276 ),
		vec3( -0.07367, -0.00605,  1.07602 )
	);
	color *= toneMappingExposure / 0.6;
	color = ACESInputMat * color;
	color = RRTAndODTFit( color );
	color = ACESOutputMat * color;
	return saturate( color );
}
const mat3 LINEAR_REC2020_TO_LINEAR_SRGB = mat3(
	vec3( 1.6605, - 0.1246, - 0.0182 ),
	vec3( - 0.5876, 1.1329, - 0.1006 ),
	vec3( - 0.0728, - 0.0083, 1.1187 )
);
const mat3 LINEAR_SRGB_TO_LINEAR_REC2020 = mat3(
	vec3( 0.6274, 0.0691, 0.0164 ),
	vec3( 0.3293, 0.9195, 0.0880 ),
	vec3( 0.0433, 0.0113, 0.8956 )
);
vec3 agxDefaultContrastApprox( vec3 x ) {
	vec3 x2 = x * x;
	vec3 x4 = x2 * x2;
	return + 15.5 * x4 * x2
		- 40.14 * x4 * x
		+ 31.96 * x4
		- 6.868 * x2 * x
		+ 0.4298 * x2
		+ 0.1191 * x
		- 0.00232;
}
vec3 AgXToneMapping( vec3 color ) {
	const mat3 AgXInsetMatrix = mat3(
		vec3( 0.856627153315983, 0.137318972929847, 0.11189821299995 ),
		vec3( 0.0951212405381588, 0.761241990602591, 0.0767994186031903 ),
		vec3( 0.0482516061458583, 0.101439036467562, 0.811302368396859 )
	);
	const mat3 AgXOutsetMatrix = mat3(
		vec3( 1.1271005818144368, - 0.1413297634984383, - 0.14132976349843826 ),
		vec3( - 0.11060664309660323, 1.157823702216272, - 0.11060664309660294 ),
		vec3( - 0.016493938717834573, - 0.016493938717834257, 1.2519364065950405 )
	);
	const float AgxMinEv = - 12.47393;	const float AgxMaxEv = 4.026069;
	color *= toneMappingExposure;
	color = LINEAR_SRGB_TO_LINEAR_REC2020 * color;
	color = AgXInsetMatrix * color;
	color = max( color, 1e-10 );	color = log2( color );
	color = ( color - AgxMinEv ) / ( AgxMaxEv - AgxMinEv );
	color = clamp( color, 0.0, 1.0 );
	color = agxDefaultContrastApprox( color );
	color = AgXOutsetMatrix * color;
	color = pow( max( vec3( 0.0 ), color ), vec3( 2.2 ) );
	color = LINEAR_REC2020_TO_LINEAR_SRGB * color;
	color = clamp( color, 0.0, 1.0 );
	return color;
}
vec3 NeutralToneMapping( vec3 color ) {
	const float StartCompression = 0.8 - 0.04;
	const float Desaturation = 0.15;
	color *= toneMappingExposure;
	float x = min( color.r, min( color.g, color.b ) );
	float offset = x < 0.08 ? x - 6.25 * x * x : 0.04;
	color -= offset;
	float peak = max( color.r, max( color.g, color.b ) );
	if ( peak < StartCompression ) return color;
	float d = 1. - StartCompression;
	float newPeak = 1. - d * d / ( peak + d - StartCompression );
	color *= newPeak / peak;
	float g = 1. - 1. / ( Desaturation * ( peak - newPeak ) + 1. );
	return mix( color, vec3( newPeak ), g );
}
vec3 CustomToneMapping( vec3 color ) { return color; }`,nf=`#ifdef USE_TRANSMISSION
	material.transmission = transmission;
	material.transmissionAlpha = 1.0;
	material.thickness = thickness;
	material.attenuationDistance = attenuationDistance;
	material.attenuationColor = attenuationColor;
	#ifdef USE_TRANSMISSIONMAP
		material.transmission *= texture2D( transmissionMap, vTransmissionMapUv ).r;
	#endif
	#ifdef USE_THICKNESSMAP
		material.thickness *= texture2D( thicknessMap, vThicknessMapUv ).g;
	#endif
	vec3 pos = vWorldPosition;
	vec3 v = normalize( cameraPosition - pos );
	vec3 n = transformNormalByInverseViewMatrix( normal, viewMatrix );
	vec4 transmitted = getIBLVolumeRefraction(
		n, v, material.roughness, material.diffuseContribution, material.specularColorBlended, material.specularF90,
		pos, modelMatrix, viewMatrix, projectionMatrix, material.dispersion, material.ior, material.thickness,
		material.attenuationColor, material.attenuationDistance );
	material.transmissionAlpha = mix( material.transmissionAlpha, transmitted.a, material.transmission );
	totalDiffuse = mix( totalDiffuse, transmitted.rgb, material.transmission );
#endif`,sf=`#ifdef USE_TRANSMISSION
	uniform float transmission;
	uniform float thickness;
	uniform float attenuationDistance;
	uniform vec3 attenuationColor;
	#ifdef USE_TRANSMISSIONMAP
		uniform sampler2D transmissionMap;
	#endif
	#ifdef USE_THICKNESSMAP
		uniform sampler2D thicknessMap;
	#endif
	uniform vec2 transmissionSamplerSize;
	uniform sampler2D transmissionSamplerMap;
	uniform mat4 modelMatrix;
	uniform mat4 projectionMatrix;
	varying vec3 vWorldPosition;
	float w0( float a ) {
		return ( 1.0 / 6.0 ) * ( a * ( a * ( - a + 3.0 ) - 3.0 ) + 1.0 );
	}
	float w1( float a ) {
		return ( 1.0 / 6.0 ) * ( a *  a * ( 3.0 * a - 6.0 ) + 4.0 );
	}
	float w2( float a ){
		return ( 1.0 / 6.0 ) * ( a * ( a * ( - 3.0 * a + 3.0 ) + 3.0 ) + 1.0 );
	}
	float w3( float a ) {
		return ( 1.0 / 6.0 ) * ( a * a * a );
	}
	float g0( float a ) {
		return w0( a ) + w1( a );
	}
	float g1( float a ) {
		return w2( a ) + w3( a );
	}
	float h0( float a ) {
		return - 1.0 + w1( a ) / ( w0( a ) + w1( a ) );
	}
	float h1( float a ) {
		return 1.0 + w3( a ) / ( w2( a ) + w3( a ) );
	}
	vec4 bicubic( sampler2D tex, vec2 uv, vec4 texelSize, float lod ) {
		uv = uv * texelSize.zw + 0.5;
		vec2 iuv = floor( uv );
		vec2 fuv = fract( uv );
		float g0x = g0( fuv.x );
		float g1x = g1( fuv.x );
		float h0x = h0( fuv.x );
		float h1x = h1( fuv.x );
		float h0y = h0( fuv.y );
		float h1y = h1( fuv.y );
		vec2 p0 = ( vec2( iuv.x + h0x, iuv.y + h0y ) - 0.5 ) * texelSize.xy;
		vec2 p1 = ( vec2( iuv.x + h1x, iuv.y + h0y ) - 0.5 ) * texelSize.xy;
		vec2 p2 = ( vec2( iuv.x + h0x, iuv.y + h1y ) - 0.5 ) * texelSize.xy;
		vec2 p3 = ( vec2( iuv.x + h1x, iuv.y + h1y ) - 0.5 ) * texelSize.xy;
		return g0( fuv.y ) * ( g0x * textureLod( tex, p0, lod ) + g1x * textureLod( tex, p1, lod ) ) +
			g1( fuv.y ) * ( g0x * textureLod( tex, p2, lod ) + g1x * textureLod( tex, p3, lod ) );
	}
	vec4 textureBicubic( sampler2D sampler, vec2 uv, float lod ) {
		vec2 fLodSize = vec2( textureSize( sampler, int( lod ) ) );
		vec2 cLodSize = vec2( textureSize( sampler, int( lod + 1.0 ) ) );
		vec2 fLodSizeInv = 1.0 / fLodSize;
		vec2 cLodSizeInv = 1.0 / cLodSize;
		vec4 fSample = bicubic( sampler, uv, vec4( fLodSizeInv, fLodSize ), floor( lod ) );
		vec4 cSample = bicubic( sampler, uv, vec4( cLodSizeInv, cLodSize ), ceil( lod ) );
		return mix( fSample, cSample, fract( lod ) );
	}
	vec3 getVolumeTransmissionRay( const in vec3 n, const in vec3 v, const in float thickness, const in float ior, const in mat4 modelMatrix ) {
		vec3 refractionVector = refract( - v, normalize( n ), 1.0 / ior );
		vec3 modelScale;
		modelScale.x = length( vec3( modelMatrix[ 0 ].xyz ) );
		modelScale.y = length( vec3( modelMatrix[ 1 ].xyz ) );
		modelScale.z = length( vec3( modelMatrix[ 2 ].xyz ) );
		return normalize( refractionVector ) * thickness * modelScale;
	}
	float applyIorToRoughness( const in float roughness, const in float ior ) {
		return roughness * clamp( ior * 2.0 - 2.0, 0.0, 1.0 );
	}
	vec4 getTransmissionSample( const in vec2 fragCoord, const in float roughness, const in float ior ) {
		float lod = log2( transmissionSamplerSize.x ) * applyIorToRoughness( roughness, ior );
		return textureBicubic( transmissionSamplerMap, fragCoord.xy, lod );
	}
	vec3 volumeAttenuation( const in float transmissionDistance, const in vec3 attenuationColor, const in float attenuationDistance ) {
		if ( isinf( attenuationDistance ) ) {
			return vec3( 1.0 );
		} else {
			vec3 attenuationCoefficient = -log( attenuationColor ) / attenuationDistance;
			vec3 transmittance = exp( - attenuationCoefficient * transmissionDistance );			return transmittance;
		}
	}
	vec4 getIBLVolumeRefraction( const in vec3 n, const in vec3 v, const in float roughness, const in vec3 diffuseColor,
		const in vec3 specularColor, const in float specularF90, const in vec3 position, const in mat4 modelMatrix,
		const in mat4 viewMatrix, const in mat4 projMatrix, const in float dispersion, const in float ior, const in float thickness,
		const in vec3 attenuationColor, const in float attenuationDistance ) {
		vec4 transmittedLight;
		vec3 transmittance;
		#ifdef USE_DISPERSION
			float halfSpread = ( ior - 1.0 ) * 0.025 * dispersion;
			vec3 iors = vec3( ior - halfSpread, ior, ior + halfSpread );
			for ( int i = 0; i < 3; i ++ ) {
				vec3 transmissionRay = getVolumeTransmissionRay( n, v, thickness, iors[ i ], modelMatrix );
				vec3 refractedRayExit = position + transmissionRay;
				vec4 ndcPos = projMatrix * viewMatrix * vec4( refractedRayExit, 1.0 );
				vec2 refractionCoords = ndcPos.xy / ndcPos.w;
				refractionCoords += 1.0;
				refractionCoords /= 2.0;
				vec4 transmissionSample = getTransmissionSample( refractionCoords, roughness, iors[ i ] );
				transmittedLight[ i ] = transmissionSample[ i ];
				transmittedLight.a += transmissionSample.a;
				transmittance[ i ] = diffuseColor[ i ] * volumeAttenuation( length( transmissionRay ), attenuationColor, attenuationDistance )[ i ];
			}
			transmittedLight.a /= 3.0;
		#else
			vec3 transmissionRay = getVolumeTransmissionRay( n, v, thickness, ior, modelMatrix );
			vec3 refractedRayExit = position + transmissionRay;
			vec4 ndcPos = projMatrix * viewMatrix * vec4( refractedRayExit, 1.0 );
			vec2 refractionCoords = ndcPos.xy / ndcPos.w;
			refractionCoords += 1.0;
			refractionCoords /= 2.0;
			transmittedLight = getTransmissionSample( refractionCoords, roughness, ior );
			transmittance = diffuseColor * volumeAttenuation( length( transmissionRay ), attenuationColor, attenuationDistance );
		#endif
		vec3 attenuatedColor = transmittance * transmittedLight.rgb;
		vec3 F = EnvironmentBRDF( n, v, specularColor, specularF90, roughness );
		float transmittanceFactor = ( transmittance.r + transmittance.g + transmittance.b ) / 3.0;
		return vec4( ( 1.0 - F ) * attenuatedColor, 1.0 - ( 1.0 - transmittedLight.a ) * transmittanceFactor );
	}
#endif`,rf=`#if defined( USE_UV ) || defined( USE_ANISOTROPY )
	varying vec2 vUv;
#endif
#ifdef USE_MAP
	varying vec2 vMapUv;
#endif
#ifdef USE_ALPHAMAP
	varying vec2 vAlphaMapUv;
#endif
#ifdef USE_LIGHTMAP
	varying vec2 vLightMapUv;
#endif
#ifdef USE_AOMAP
	varying vec2 vAoMapUv;
#endif
#ifdef USE_BUMPMAP
	varying vec2 vBumpMapUv;
#endif
#ifdef USE_NORMALMAP
	varying vec2 vNormalMapUv;
#endif
#ifdef USE_EMISSIVEMAP
	varying vec2 vEmissiveMapUv;
#endif
#ifdef USE_METALNESSMAP
	varying vec2 vMetalnessMapUv;
#endif
#ifdef USE_ROUGHNESSMAP
	varying vec2 vRoughnessMapUv;
#endif
#ifdef USE_DIFFUSE_ROUGHNESSMAP
	varying vec2 vDiffuseRoughnessMapUv;
#endif
#ifdef USE_ANISOTROPYMAP
	varying vec2 vAnisotropyMapUv;
#endif
#ifdef USE_CLEARCOATMAP
	varying vec2 vClearcoatMapUv;
#endif
#ifdef USE_CLEARCOAT_NORMALMAP
	varying vec2 vClearcoatNormalMapUv;
#endif
#ifdef USE_CLEARCOAT_ROUGHNESSMAP
	varying vec2 vClearcoatRoughnessMapUv;
#endif
#ifdef USE_IRIDESCENCEMAP
	varying vec2 vIridescenceMapUv;
#endif
#ifdef USE_IRIDESCENCE_THICKNESSMAP
	varying vec2 vIridescenceThicknessMapUv;
#endif
#ifdef USE_SHEEN_COLORMAP
	varying vec2 vSheenColorMapUv;
#endif
#ifdef USE_SHEEN_ROUGHNESSMAP
	varying vec2 vSheenRoughnessMapUv;
#endif
#ifdef USE_SPECULARMAP
	varying vec2 vSpecularMapUv;
#endif
#ifdef USE_SPECULAR_COLORMAP
	varying vec2 vSpecularColorMapUv;
#endif
#ifdef USE_SPECULAR_INTENSITYMAP
	varying vec2 vSpecularIntensityMapUv;
#endif
#ifdef USE_TRANSMISSIONMAP
	uniform mat3 transmissionMapTransform;
	varying vec2 vTransmissionMapUv;
#endif
#ifdef USE_THICKNESSMAP
	uniform mat3 thicknessMapTransform;
	varying vec2 vThicknessMapUv;
#endif`,af=`#if defined( USE_UV ) || defined( USE_ANISOTROPY )
	varying vec2 vUv;
#endif
#ifdef USE_MAP
	uniform mat3 mapTransform;
	varying vec2 vMapUv;
#endif
#ifdef USE_ALPHAMAP
	uniform mat3 alphaMapTransform;
	varying vec2 vAlphaMapUv;
#endif
#ifdef USE_LIGHTMAP
	uniform mat3 lightMapTransform;
	varying vec2 vLightMapUv;
#endif
#ifdef USE_AOMAP
	uniform mat3 aoMapTransform;
	varying vec2 vAoMapUv;
#endif
#ifdef USE_BUMPMAP
	uniform mat3 bumpMapTransform;
	varying vec2 vBumpMapUv;
#endif
#ifdef USE_NORMALMAP
	uniform mat3 normalMapTransform;
	varying vec2 vNormalMapUv;
#endif
#ifdef USE_DISPLACEMENTMAP
	uniform mat3 displacementMapTransform;
	varying vec2 vDisplacementMapUv;
#endif
#ifdef USE_EMISSIVEMAP
	uniform mat3 emissiveMapTransform;
	varying vec2 vEmissiveMapUv;
#endif
#ifdef USE_METALNESSMAP
	uniform mat3 metalnessMapTransform;
	varying vec2 vMetalnessMapUv;
#endif
#ifdef USE_ROUGHNESSMAP
	uniform mat3 roughnessMapTransform;
	varying vec2 vRoughnessMapUv;
#endif
#ifdef USE_DIFFUSE_ROUGHNESSMAP
	uniform mat3 diffuseRoughnessMapTransform;
	varying vec2 vDiffuseRoughnessMapUv;
#endif
#ifdef USE_ANISOTROPYMAP
	uniform mat3 anisotropyMapTransform;
	varying vec2 vAnisotropyMapUv;
#endif
#ifdef USE_CLEARCOATMAP
	uniform mat3 clearcoatMapTransform;
	varying vec2 vClearcoatMapUv;
#endif
#ifdef USE_CLEARCOAT_NORMALMAP
	uniform mat3 clearcoatNormalMapTransform;
	varying vec2 vClearcoatNormalMapUv;
#endif
#ifdef USE_CLEARCOAT_ROUGHNESSMAP
	uniform mat3 clearcoatRoughnessMapTransform;
	varying vec2 vClearcoatRoughnessMapUv;
#endif
#ifdef USE_SHEEN_COLORMAP
	uniform mat3 sheenColorMapTransform;
	varying vec2 vSheenColorMapUv;
#endif
#ifdef USE_SHEEN_ROUGHNESSMAP
	uniform mat3 sheenRoughnessMapTransform;
	varying vec2 vSheenRoughnessMapUv;
#endif
#ifdef USE_IRIDESCENCEMAP
	uniform mat3 iridescenceMapTransform;
	varying vec2 vIridescenceMapUv;
#endif
#ifdef USE_IRIDESCENCE_THICKNESSMAP
	uniform mat3 iridescenceThicknessMapTransform;
	varying vec2 vIridescenceThicknessMapUv;
#endif
#ifdef USE_SPECULARMAP
	uniform mat3 specularMapTransform;
	varying vec2 vSpecularMapUv;
#endif
#ifdef USE_SPECULAR_COLORMAP
	uniform mat3 specularColorMapTransform;
	varying vec2 vSpecularColorMapUv;
#endif
#ifdef USE_SPECULAR_INTENSITYMAP
	uniform mat3 specularIntensityMapTransform;
	varying vec2 vSpecularIntensityMapUv;
#endif
#ifdef USE_TRANSMISSIONMAP
	uniform mat3 transmissionMapTransform;
	varying vec2 vTransmissionMapUv;
#endif
#ifdef USE_THICKNESSMAP
	uniform mat3 thicknessMapTransform;
	varying vec2 vThicknessMapUv;
#endif`,of=`#if defined( USE_UV ) || defined( USE_ANISOTROPY )
	vUv = vec3( uv, 1 ).xy;
#endif
#ifdef USE_MAP
	vMapUv = ( mapTransform * vec3( MAP_UV, 1 ) ).xy;
#endif
#ifdef USE_ALPHAMAP
	vAlphaMapUv = ( alphaMapTransform * vec3( ALPHAMAP_UV, 1 ) ).xy;
#endif
#ifdef USE_LIGHTMAP
	vLightMapUv = ( lightMapTransform * vec3( LIGHTMAP_UV, 1 ) ).xy;
#endif
#ifdef USE_AOMAP
	vAoMapUv = ( aoMapTransform * vec3( AOMAP_UV, 1 ) ).xy;
#endif
#ifdef USE_BUMPMAP
	vBumpMapUv = ( bumpMapTransform * vec3( BUMPMAP_UV, 1 ) ).xy;
#endif
#ifdef USE_NORMALMAP
	vNormalMapUv = ( normalMapTransform * vec3( NORMALMAP_UV, 1 ) ).xy;
#endif
#ifdef USE_DISPLACEMENTMAP
	vDisplacementMapUv = ( displacementMapTransform * vec3( DISPLACEMENTMAP_UV, 1 ) ).xy;
#endif
#ifdef USE_EMISSIVEMAP
	vEmissiveMapUv = ( emissiveMapTransform * vec3( EMISSIVEMAP_UV, 1 ) ).xy;
#endif
#ifdef USE_METALNESSMAP
	vMetalnessMapUv = ( metalnessMapTransform * vec3( METALNESSMAP_UV, 1 ) ).xy;
#endif
#ifdef USE_ROUGHNESSMAP
	vRoughnessMapUv = ( roughnessMapTransform * vec3( ROUGHNESSMAP_UV, 1 ) ).xy;
#endif
#ifdef USE_DIFFUSE_ROUGHNESSMAP
	vDiffuseRoughnessMapUv = ( diffuseRoughnessMapTransform * vec3( DIFFUSE_ROUGHNESSMAP_UV, 1 ) ).xy;
#endif
#ifdef USE_ANISOTROPYMAP
	vAnisotropyMapUv = ( anisotropyMapTransform * vec3( ANISOTROPYMAP_UV, 1 ) ).xy;
#endif
#ifdef USE_CLEARCOATMAP
	vClearcoatMapUv = ( clearcoatMapTransform * vec3( CLEARCOATMAP_UV, 1 ) ).xy;
#endif
#ifdef USE_CLEARCOAT_NORMALMAP
	vClearcoatNormalMapUv = ( clearcoatNormalMapTransform * vec3( CLEARCOAT_NORMALMAP_UV, 1 ) ).xy;
#endif
#ifdef USE_CLEARCOAT_ROUGHNESSMAP
	vClearcoatRoughnessMapUv = ( clearcoatRoughnessMapTransform * vec3( CLEARCOAT_ROUGHNESSMAP_UV, 1 ) ).xy;
#endif
#ifdef USE_IRIDESCENCEMAP
	vIridescenceMapUv = ( iridescenceMapTransform * vec3( IRIDESCENCEMAP_UV, 1 ) ).xy;
#endif
#ifdef USE_IRIDESCENCE_THICKNESSMAP
	vIridescenceThicknessMapUv = ( iridescenceThicknessMapTransform * vec3( IRIDESCENCE_THICKNESSMAP_UV, 1 ) ).xy;
#endif
#ifdef USE_SHEEN_COLORMAP
	vSheenColorMapUv = ( sheenColorMapTransform * vec3( SHEEN_COLORMAP_UV, 1 ) ).xy;
#endif
#ifdef USE_SHEEN_ROUGHNESSMAP
	vSheenRoughnessMapUv = ( sheenRoughnessMapTransform * vec3( SHEEN_ROUGHNESSMAP_UV, 1 ) ).xy;
#endif
#ifdef USE_SPECULARMAP
	vSpecularMapUv = ( specularMapTransform * vec3( SPECULARMAP_UV, 1 ) ).xy;
#endif
#ifdef USE_SPECULAR_COLORMAP
	vSpecularColorMapUv = ( specularColorMapTransform * vec3( SPECULAR_COLORMAP_UV, 1 ) ).xy;
#endif
#ifdef USE_SPECULAR_INTENSITYMAP
	vSpecularIntensityMapUv = ( specularIntensityMapTransform * vec3( SPECULAR_INTENSITYMAP_UV, 1 ) ).xy;
#endif
#ifdef USE_TRANSMISSIONMAP
	vTransmissionMapUv = ( transmissionMapTransform * vec3( TRANSMISSIONMAP_UV, 1 ) ).xy;
#endif
#ifdef USE_THICKNESSMAP
	vThicknessMapUv = ( thicknessMapTransform * vec3( THICKNESSMAP_UV, 1 ) ).xy;
#endif`;var lf=`#if defined( USE_ENVMAP ) || defined( DISTANCE ) || defined ( USE_SHADOWMAP ) || defined ( USE_TRANSMISSION ) || NUM_SPOT_LIGHT_COORDS > 0
	vec4 worldPosition = vec4( transformed, 1.0 );
	#ifdef USE_BATCHING
		worldPosition = batchingMatrix * worldPosition;
	#endif
	#ifdef USE_INSTANCING
		worldPosition = instanceMatrix * worldPosition;
	#endif
	worldPosition = modelMatrix * worldPosition;
#endif`,cf=`varying vec2 vUv;
uniform mat3 uvTransform;
void main() {
	vUv = ( uvTransform * vec3( uv, 1 ) ).xy;
	gl_Position = vec4( position.xy, 1.0, 1.0 );
}`,hf=`uniform sampler2D t2D;
uniform float backgroundIntensity;
varying vec2 vUv;
void main() {
	vec4 texColor = texture2D( t2D, vUv );
	#ifdef DECODE_VIDEO_TEXTURE
		texColor = vec4( mix( pow( texColor.rgb * 0.9478672986 + vec3( 0.0521327014 ), vec3( 2.4 ) ), texColor.rgb * 0.0773993808, vec3( lessThanEqual( texColor.rgb, vec3( 0.04045 ) ) ) ), texColor.w );
	#endif
	texColor.rgb *= backgroundIntensity;
	gl_FragColor = texColor;
	#include <tonemapping_fragment>
	#include <colorspace_fragment>
}`,uf=`varying vec3 vWorldDirection;
#include <common>
void main() {
	vWorldDirection = transformDirection( position, modelMatrix );
	#include <begin_vertex>
	#include <project_vertex>
	gl_Position.z = gl_Position.w;
}`,df=`uniform float backgroundBlurriness;
uniform float backgroundIntensity;
uniform mat3 backgroundRotation;
varying vec3 vWorldDirection;
#include <envmap_common_pars_fragment>
void main() {
	#ifdef ENVMAP_TYPE_PMREM
		vec4 texColor = textureLod( envMap, backgroundRotation * vWorldDirection, roughnessToMip( backgroundBlurriness ) );
	#else
		vec4 texColor = textureCube( envMap, backgroundRotation * vWorldDirection );
	#endif
	texColor.rgb *= backgroundIntensity;
	gl_FragColor = texColor;
	#include <tonemapping_fragment>
	#include <colorspace_fragment>
}`,ff=`varying vec3 vWorldDirection;
#include <common>
void main() {
	vWorldDirection = transformDirection( position, modelMatrix );
	#include <begin_vertex>
	#include <project_vertex>
	gl_Position.z = gl_Position.w;
}`,pf=`uniform samplerCube tCube;
uniform float tFlip;
uniform float opacity;
varying vec3 vWorldDirection;
void main() {
	vec4 texColor = textureCube( tCube, vec3( tFlip * vWorldDirection.x, vWorldDirection.yz ) );
	gl_FragColor = texColor;
	gl_FragColor.a *= opacity;
	#include <tonemapping_fragment>
	#include <colorspace_fragment>
}`,mf=`#include <common>
#include <batching_pars_vertex>
#include <uv_pars_vertex>
#include <displacementmap_pars_vertex>
#include <morphtarget_pars_vertex>
#include <skinning_pars_vertex>
#include <logdepthbuf_pars_vertex>
#include <clipping_planes_pars_vertex>
varying vec2 vHighPrecisionZW;
void main() {
	#include <uv_vertex>
	#include <batching_vertex>
	#include <skinbase_vertex>
	#include <morphinstance_vertex>
	#ifdef USE_DISPLACEMENTMAP
		#include <beginnormal_vertex>
		#include <morphnormal_vertex>
		#include <skinnormal_vertex>
	#endif
	#include <begin_vertex>
	#include <morphtarget_vertex>
	#include <skinning_vertex>
	#include <displacementmap_vertex>
	#include <project_vertex>
	#include <logdepthbuf_vertex>
	#include <clipping_planes_vertex>
	vHighPrecisionZW = gl_Position.zw;
}`,gf=`#if DEPTH_PACKING == 3200
	uniform float opacity;
#endif
#include <common>
#include <packing>
#include <uv_pars_fragment>
#include <map_pars_fragment>
#include <alphamap_pars_fragment>
#include <alphatest_pars_fragment>
#include <alphahash_pars_fragment>
#include <logdepthbuf_pars_fragment>
#include <clipping_planes_pars_fragment>
varying vec2 vHighPrecisionZW;
void main() {
	vec4 diffuseColor = vec4( 1.0 );
	#include <clipping_planes_fragment>
	#if DEPTH_PACKING == 3200
		diffuseColor.a = opacity;
	#endif
	#include <map_fragment>
	#include <alphamap_fragment>
	#include <alphatest_fragment>
	#include <alphahash_fragment>
	#include <logdepthbuf_fragment>
	#ifdef USE_REVERSED_DEPTH_BUFFER
		float fragCoordZ = vHighPrecisionZW[ 0 ] / vHighPrecisionZW[ 1 ];
	#else
		float fragCoordZ = 0.5 * vHighPrecisionZW[ 0 ] / vHighPrecisionZW[ 1 ] + 0.5;
	#endif
	#if DEPTH_PACKING == 3200
		gl_FragColor = vec4( vec3( 1.0 - fragCoordZ ), opacity );
	#elif DEPTH_PACKING == 3201
		gl_FragColor = packDepthToRGBA( fragCoordZ );
	#elif DEPTH_PACKING == 3202
		gl_FragColor = vec4( packDepthToRGB( fragCoordZ ), 1.0 );
	#elif DEPTH_PACKING == 3203
		gl_FragColor = vec4( packDepthToRG( fragCoordZ ), 0.0, 1.0 );
	#endif
}`,_f=`#define DISTANCE
varying vec3 vWorldPosition;
#include <common>
#include <batching_pars_vertex>
#include <uv_pars_vertex>
#include <displacementmap_pars_vertex>
#include <morphtarget_pars_vertex>
#include <skinning_pars_vertex>
#include <clipping_planes_pars_vertex>
void main() {
	#include <uv_vertex>
	#include <batching_vertex>
	#include <skinbase_vertex>
	#include <morphinstance_vertex>
	#ifdef USE_DISPLACEMENTMAP
		#include <beginnormal_vertex>
		#include <morphnormal_vertex>
		#include <skinnormal_vertex>
	#endif
	#include <begin_vertex>
	#include <morphtarget_vertex>
	#include <skinning_vertex>
	#include <displacementmap_vertex>
	#include <project_vertex>
	#include <worldpos_vertex>
	#include <clipping_planes_vertex>
	vWorldPosition = worldPosition.xyz;
}`,xf=`#define DISTANCE
uniform vec3 referencePosition;
uniform float nearDistance;
uniform float farDistance;
varying vec3 vWorldPosition;
#include <common>
#include <uv_pars_fragment>
#include <map_pars_fragment>
#include <alphamap_pars_fragment>
#include <alphatest_pars_fragment>
#include <alphahash_pars_fragment>
#include <clipping_planes_pars_fragment>
void main() {
	vec4 diffuseColor = vec4( 1.0 );
	#include <clipping_planes_fragment>
	#include <map_fragment>
	#include <alphamap_fragment>
	#include <alphatest_fragment>
	#include <alphahash_fragment>
	float dist = length( vWorldPosition - referencePosition );
	dist = ( dist - nearDistance ) / ( farDistance - nearDistance );
	dist = saturate( dist );
	gl_FragColor = vec4( dist, 0.0, 0.0, 1.0 );
}`,vf=`varying vec3 vWorldDirection;
#include <common>
void main() {
	vWorldDirection = transformDirection( position, modelMatrix );
	#include <begin_vertex>
	#include <project_vertex>
}`,yf=`uniform sampler2D tEquirect;
varying vec3 vWorldDirection;
#include <common>
void main() {
	vec3 direction = normalize( vWorldDirection );
	vec2 sampleUV = equirectUv( direction );
	gl_FragColor = texture2D( tEquirect, sampleUV );
	#include <tonemapping_fragment>
	#include <colorspace_fragment>
}`,Mf=`uniform float scale;
attribute float lineDistance;
varying float vLineDistance;
#include <common>
#include <uv_pars_vertex>
#include <color_pars_vertex>
#include <fog_pars_vertex>
#include <morphtarget_pars_vertex>
#include <logdepthbuf_pars_vertex>
#include <clipping_planes_pars_vertex>
void main() {
	vLineDistance = scale * lineDistance;
	#include <uv_vertex>
	#include <color_vertex>
	#include <morphinstance_vertex>
	#include <morphcolor_vertex>
	#include <begin_vertex>
	#include <morphtarget_vertex>
	#include <project_vertex>
	#include <logdepthbuf_vertex>
	#include <clipping_planes_vertex>
	#include <fog_vertex>
}`,Sf=`uniform vec3 diffuse;
uniform float opacity;
uniform float dashSize;
uniform float totalSize;
varying float vLineDistance;
#include <common>
#include <color_pars_fragment>
#include <uv_pars_fragment>
#include <map_pars_fragment>
#include <fog_pars_fragment>
#include <logdepthbuf_pars_fragment>
#include <clipping_planes_pars_fragment>
void main() {
	vec4 diffuseColor = vec4( diffuse, opacity );
	#include <clipping_planes_fragment>
	if ( mod( vLineDistance, totalSize ) > dashSize ) {
		discard;
	}
	vec3 outgoingLight = vec3( 0.0 );
	#include <logdepthbuf_fragment>
	#include <map_fragment>
	#include <color_fragment>
	outgoingLight = diffuseColor.rgb;
	#include <opaque_fragment>
	#include <tonemapping_fragment>
	#include <colorspace_fragment>
	#include <fog_fragment>
	#include <premultiplied_alpha_fragment>
}`,bf=`#include <common>
#include <batching_pars_vertex>
#include <uv_pars_vertex>
#include <envmap_pars_vertex>
#include <color_pars_vertex>
#include <fog_pars_vertex>
#include <morphtarget_pars_vertex>
#include <skinning_pars_vertex>
#include <logdepthbuf_pars_vertex>
#include <clipping_planes_pars_vertex>
void main() {
	#include <uv_vertex>
	#include <color_vertex>
	#include <morphinstance_vertex>
	#include <morphcolor_vertex>
	#include <batching_vertex>
	#if defined ( USE_ENVMAP ) || defined ( USE_SKINNING )
		#include <beginnormal_vertex>
		#include <morphnormal_vertex>
		#include <skinbase_vertex>
		#include <skinnormal_vertex>
		#include <defaultnormal_vertex>
	#endif
	#include <begin_vertex>
	#include <morphtarget_vertex>
	#include <skinning_vertex>
	#include <project_vertex>
	#include <logdepthbuf_vertex>
	#include <clipping_planes_vertex>
	#include <worldpos_vertex>
	#include <envmap_vertex>
	#include <fog_vertex>
}`,wf=`uniform vec3 diffuse;
uniform float opacity;
#ifndef FLAT_SHADED
	varying vec3 vNormal;
#endif
#include <common>
#include <dithering_pars_fragment>
#include <color_pars_fragment>
#include <uv_pars_fragment>
#include <map_pars_fragment>
#include <alphamap_pars_fragment>
#include <alphatest_pars_fragment>
#include <alphahash_pars_fragment>
#include <aomap_pars_fragment>
#include <lightmap_pars_fragment>
#include <envmap_common_pars_fragment>
#include <envmap_pars_fragment>
#include <fog_pars_fragment>
#include <specularmap_pars_fragment>
#include <logdepthbuf_pars_fragment>
#include <clipping_planes_pars_fragment>
void main() {
	vec4 diffuseColor = vec4( diffuse, opacity );
	#include <clipping_planes_fragment>
	#include <logdepthbuf_fragment>
	#include <map_fragment>
	#include <color_fragment>
	#include <alphamap_fragment>
	#include <alphatest_fragment>
	#include <alphahash_fragment>
	#include <specularmap_fragment>
	ReflectedLight reflectedLight = ReflectedLight( vec3( 0.0 ), vec3( 0.0 ), vec3( 0.0 ), vec3( 0.0 ) );
	#ifdef USE_LIGHTMAP
		vec4 lightMapTexel = texture2D( lightMap, vLightMapUv );
		reflectedLight.indirectDiffuse += lightMapTexel.rgb * lightMapIntensity * RECIPROCAL_PI;
	#else
		reflectedLight.indirectDiffuse += vec3( 1.0 );
	#endif
	#include <aomap_fragment>
	reflectedLight.indirectDiffuse *= diffuseColor.rgb;
	vec3 outgoingLight = reflectedLight.indirectDiffuse;
	#include <envmap_fragment>
	#include <opaque_fragment>
	#include <tonemapping_fragment>
	#include <colorspace_fragment>
	#include <fog_fragment>
	#include <premultiplied_alpha_fragment>
	#include <dithering_fragment>
}`,Af=`#define LAMBERT
varying vec3 vViewPosition;
#include <common>
#include <batching_pars_vertex>
#include <uv_pars_vertex>
#include <displacementmap_pars_vertex>
#include <envmap_pars_vertex>
#include <color_pars_vertex>
#include <fog_pars_vertex>
#include <normal_pars_vertex>
#include <morphtarget_pars_vertex>
#include <skinning_pars_vertex>
#include <shadowmap_pars_vertex>
#include <logdepthbuf_pars_vertex>
#include <clipping_planes_pars_vertex>
void main() {
	#include <uv_vertex>
	#include <color_vertex>
	#include <morphinstance_vertex>
	#include <morphcolor_vertex>
	#include <batching_vertex>
	#include <beginnormal_vertex>
	#include <morphnormal_vertex>
	#include <skinbase_vertex>
	#include <skinnormal_vertex>
	#include <defaultnormal_vertex>
	#include <normal_vertex>
	#include <begin_vertex>
	#include <morphtarget_vertex>
	#include <skinning_vertex>
	#include <displacementmap_vertex>
	#include <project_vertex>
	#include <logdepthbuf_vertex>
	#include <clipping_planes_vertex>
	vViewPosition = - mvPosition.xyz;
	#include <worldpos_vertex>
	#include <envmap_vertex>
	#include <shadowmap_vertex>
	#include <fog_vertex>
}`,Tf=`#define LAMBERT
uniform vec3 diffuse;
uniform vec3 emissive;
uniform float opacity;
#include <common>
#include <dithering_pars_fragment>
#include <color_pars_fragment>
#include <uv_pars_fragment>
#include <map_pars_fragment>
#include <alphamap_pars_fragment>
#include <alphatest_pars_fragment>
#include <alphahash_pars_fragment>
#include <aomap_pars_fragment>
#include <lightmap_pars_fragment>
#include <emissivemap_pars_fragment>
#include <envmap_common_pars_fragment>
#include <envmap_pars_fragment>
#include <envmap_physical_pars_fragment>
#include <fog_pars_fragment>
#include <bsdfs>
#include <lights_pars_begin>
#include <normal_pars_fragment>
#include <lights_lambert_pars_fragment>
#include <shadowmap_pars_fragment>
#include <bumpmap_pars_fragment>
#include <normalmap_pars_fragment>
#include <specularmap_pars_fragment>
#include <logdepthbuf_pars_fragment>
#include <clipping_planes_pars_fragment>
void main() {
	vec4 diffuseColor = vec4( diffuse, opacity );
	#include <clipping_planes_fragment>
	ReflectedLight reflectedLight = ReflectedLight( vec3( 0.0 ), vec3( 0.0 ), vec3( 0.0 ), vec3( 0.0 ) );
	vec3 totalEmissiveRadiance = emissive;
	#include <logdepthbuf_fragment>
	#include <map_fragment>
	#include <color_fragment>
	#include <alphamap_fragment>
	#include <alphatest_fragment>
	#include <alphahash_fragment>
	#include <specularmap_fragment>
	#include <normal_fragment_begin>
	#include <normal_fragment_maps>
	#include <emissivemap_fragment>
	#include <lights_lambert_fragment>
	#include <lights_fragment_begin>
	#include <lights_fragment_maps>
	#include <lights_fragment_end>
	#include <aomap_fragment>
	vec3 outgoingLight = reflectedLight.directDiffuse + reflectedLight.indirectDiffuse + totalEmissiveRadiance;
	#include <envmap_fragment>
	#include <opaque_fragment>
	#include <tonemapping_fragment>
	#include <colorspace_fragment>
	#include <fog_fragment>
	#include <premultiplied_alpha_fragment>
	#include <dithering_fragment>
}`,Ef=`#define MATCAP
varying vec3 vViewPosition;
#include <common>
#include <batching_pars_vertex>
#include <uv_pars_vertex>
#include <color_pars_vertex>
#include <displacementmap_pars_vertex>
#include <fog_pars_vertex>
#include <normal_pars_vertex>
#include <morphtarget_pars_vertex>
#include <skinning_pars_vertex>
#include <logdepthbuf_pars_vertex>
#include <clipping_planes_pars_vertex>
void main() {
	#include <uv_vertex>
	#include <color_vertex>
	#include <morphinstance_vertex>
	#include <morphcolor_vertex>
	#include <batching_vertex>
	#include <beginnormal_vertex>
	#include <morphnormal_vertex>
	#include <skinbase_vertex>
	#include <skinnormal_vertex>
	#include <defaultnormal_vertex>
	#include <normal_vertex>
	#include <begin_vertex>
	#include <morphtarget_vertex>
	#include <skinning_vertex>
	#include <displacementmap_vertex>
	#include <project_vertex>
	#include <logdepthbuf_vertex>
	#include <clipping_planes_vertex>
	#include <fog_vertex>
	vViewPosition = - mvPosition.xyz;
}`,Rf=`#define MATCAP
uniform vec3 diffuse;
uniform float opacity;
uniform sampler2D matcap;
varying vec3 vViewPosition;
#include <common>
#include <dithering_pars_fragment>
#include <color_pars_fragment>
#include <uv_pars_fragment>
#include <map_pars_fragment>
#include <alphamap_pars_fragment>
#include <alphatest_pars_fragment>
#include <alphahash_pars_fragment>
#include <fog_pars_fragment>
#include <normal_pars_fragment>
#include <bumpmap_pars_fragment>
#include <normalmap_pars_fragment>
#include <logdepthbuf_pars_fragment>
#include <clipping_planes_pars_fragment>
void main() {
	vec4 diffuseColor = vec4( diffuse, opacity );
	#include <clipping_planes_fragment>
	#include <logdepthbuf_fragment>
	#include <map_fragment>
	#include <color_fragment>
	#include <alphamap_fragment>
	#include <alphatest_fragment>
	#include <alphahash_fragment>
	#include <normal_fragment_begin>
	#include <normal_fragment_maps>
	vec3 viewDir = normalize( vViewPosition );
	vec3 x = normalize( vec3( viewDir.z, 0.0, - viewDir.x ) );
	vec3 y = cross( viewDir, x );
	vec2 uv = vec2( dot( x, normal ), dot( y, normal ) ) * 0.495 + 0.5;
	#ifdef USE_MATCAP
		vec4 matcapColor = texture2D( matcap, uv );
	#else
		vec4 matcapColor = vec4( vec3( mix( 0.2, 0.8, uv.y ) ), 1.0 );
	#endif
	vec3 outgoingLight = diffuseColor.rgb * matcapColor.rgb;
	#include <opaque_fragment>
	#include <tonemapping_fragment>
	#include <colorspace_fragment>
	#include <fog_fragment>
	#include <premultiplied_alpha_fragment>
	#include <dithering_fragment>
}`,Cf=`#define NORMAL
#if defined( FLAT_SHADED ) || defined( USE_BUMPMAP ) || defined( USE_NORMALMAP_TANGENTSPACE )
	varying vec3 vViewPosition;
#endif
#include <common>
#include <batching_pars_vertex>
#include <uv_pars_vertex>
#include <displacementmap_pars_vertex>
#include <normal_pars_vertex>
#include <morphtarget_pars_vertex>
#include <skinning_pars_vertex>
#include <logdepthbuf_pars_vertex>
#include <clipping_planes_pars_vertex>
void main() {
	#include <uv_vertex>
	#include <batching_vertex>
	#include <beginnormal_vertex>
	#include <morphinstance_vertex>
	#include <morphnormal_vertex>
	#include <skinbase_vertex>
	#include <skinnormal_vertex>
	#include <defaultnormal_vertex>
	#include <normal_vertex>
	#include <begin_vertex>
	#include <morphtarget_vertex>
	#include <skinning_vertex>
	#include <displacementmap_vertex>
	#include <project_vertex>
	#include <logdepthbuf_vertex>
	#include <clipping_planes_vertex>
#if defined( FLAT_SHADED ) || defined( USE_BUMPMAP ) || defined( USE_NORMALMAP_TANGENTSPACE )
	vViewPosition = - mvPosition.xyz;
#endif
}`,If=`#define NORMAL
uniform float opacity;
#if defined( FLAT_SHADED ) || defined( USE_BUMPMAP ) || defined( USE_NORMALMAP_TANGENTSPACE )
	varying vec3 vViewPosition;
#endif
#include <uv_pars_fragment>
#include <normal_pars_fragment>
#include <bumpmap_pars_fragment>
#include <normalmap_pars_fragment>
#include <logdepthbuf_pars_fragment>
#include <clipping_planes_pars_fragment>
void main() {
	vec4 diffuseColor = vec4( 0.0, 0.0, 0.0, opacity );
	#include <clipping_planes_fragment>
	#include <logdepthbuf_fragment>
	#include <normal_fragment_begin>
	#include <normal_fragment_maps>
	gl_FragColor = vec4( normalize( normal ) * 0.5 + 0.5, diffuseColor.a );
	#ifdef OPAQUE
		gl_FragColor.a = 1.0;
	#endif
}`,Pf=`#define PHONG
varying vec3 vViewPosition;
#include <common>
#include <batching_pars_vertex>
#include <uv_pars_vertex>
#include <displacementmap_pars_vertex>
#include <envmap_pars_vertex>
#include <color_pars_vertex>
#include <fog_pars_vertex>
#include <normal_pars_vertex>
#include <morphtarget_pars_vertex>
#include <skinning_pars_vertex>
#include <shadowmap_pars_vertex>
#include <logdepthbuf_pars_vertex>
#include <clipping_planes_pars_vertex>
void main() {
	#include <uv_vertex>
	#include <color_vertex>
	#include <morphcolor_vertex>
	#include <batching_vertex>
	#include <beginnormal_vertex>
	#include <morphinstance_vertex>
	#include <morphnormal_vertex>
	#include <skinbase_vertex>
	#include <skinnormal_vertex>
	#include <defaultnormal_vertex>
	#include <normal_vertex>
	#include <begin_vertex>
	#include <morphtarget_vertex>
	#include <skinning_vertex>
	#include <displacementmap_vertex>
	#include <project_vertex>
	#include <logdepthbuf_vertex>
	#include <clipping_planes_vertex>
	vViewPosition = - mvPosition.xyz;
	#include <worldpos_vertex>
	#include <envmap_vertex>
	#include <shadowmap_vertex>
	#include <fog_vertex>
}`,Lf=`#define PHONG
uniform vec3 diffuse;
uniform vec3 emissive;
uniform vec3 specular;
uniform float shininess;
uniform float opacity;
#include <common>
#include <dithering_pars_fragment>
#include <color_pars_fragment>
#include <uv_pars_fragment>
#include <map_pars_fragment>
#include <alphamap_pars_fragment>
#include <alphatest_pars_fragment>
#include <alphahash_pars_fragment>
#include <aomap_pars_fragment>
#include <lightmap_pars_fragment>
#include <emissivemap_pars_fragment>
#include <envmap_common_pars_fragment>
#include <envmap_pars_fragment>
#include <envmap_physical_pars_fragment>
#include <fog_pars_fragment>
#include <bsdfs>
#include <lights_pars_begin>
#include <normal_pars_fragment>
#include <lights_phong_pars_fragment>
#include <shadowmap_pars_fragment>
#include <bumpmap_pars_fragment>
#include <normalmap_pars_fragment>
#include <specularmap_pars_fragment>
#include <logdepthbuf_pars_fragment>
#include <clipping_planes_pars_fragment>
void main() {
	vec4 diffuseColor = vec4( diffuse, opacity );
	#include <clipping_planes_fragment>
	ReflectedLight reflectedLight = ReflectedLight( vec3( 0.0 ), vec3( 0.0 ), vec3( 0.0 ), vec3( 0.0 ) );
	vec3 totalEmissiveRadiance = emissive;
	#include <logdepthbuf_fragment>
	#include <map_fragment>
	#include <color_fragment>
	#include <alphamap_fragment>
	#include <alphatest_fragment>
	#include <alphahash_fragment>
	#include <specularmap_fragment>
	#include <normal_fragment_begin>
	#include <normal_fragment_maps>
	#include <emissivemap_fragment>
	#include <lights_phong_fragment>
	#include <lights_fragment_begin>
	#include <lights_fragment_maps>
	#include <lights_fragment_end>
	#include <aomap_fragment>
	vec3 outgoingLight = reflectedLight.directDiffuse + reflectedLight.indirectDiffuse + reflectedLight.directSpecular + reflectedLight.indirectSpecular + totalEmissiveRadiance;
	#include <envmap_fragment>
	#include <opaque_fragment>
	#include <tonemapping_fragment>
	#include <colorspace_fragment>
	#include <fog_fragment>
	#include <premultiplied_alpha_fragment>
	#include <dithering_fragment>
}`,Nf=`#define STANDARD
varying vec3 vViewPosition;
#ifdef USE_TRANSMISSION
	varying vec3 vWorldPosition;
#endif
#include <common>
#include <batching_pars_vertex>
#include <uv_pars_vertex>
#include <displacementmap_pars_vertex>
#include <color_pars_vertex>
#include <fog_pars_vertex>
#include <normal_pars_vertex>
#include <morphtarget_pars_vertex>
#include <skinning_pars_vertex>
#include <shadowmap_pars_vertex>
#include <logdepthbuf_pars_vertex>
#include <clipping_planes_pars_vertex>
void main() {
	#include <uv_vertex>
	#include <color_vertex>
	#include <morphinstance_vertex>
	#include <morphcolor_vertex>
	#include <batching_vertex>
	#include <beginnormal_vertex>
	#include <morphnormal_vertex>
	#include <skinbase_vertex>
	#include <skinnormal_vertex>
	#include <defaultnormal_vertex>
	#include <normal_vertex>
	#include <begin_vertex>
	#include <morphtarget_vertex>
	#include <skinning_vertex>
	#include <displacementmap_vertex>
	#include <project_vertex>
	#include <logdepthbuf_vertex>
	#include <clipping_planes_vertex>
	vViewPosition = - mvPosition.xyz;
	#include <worldpos_vertex>
	#include <shadowmap_vertex>
	#include <fog_vertex>
#ifdef USE_TRANSMISSION
	vWorldPosition = worldPosition.xyz;
#endif
}`,Df=`#define STANDARD
#ifdef PHYSICAL
	#define IOR
	#define USE_SPECULAR
#endif
uniform vec3 diffuse;
uniform vec3 emissive;
uniform float roughness;
uniform float metalness;
uniform float opacity;
#ifdef IOR
	uniform float ior;
#endif
#ifdef USE_SPECULAR
	uniform float specularIntensity;
	uniform vec3 specularColor;
	#ifdef USE_SPECULAR_COLORMAP
		uniform sampler2D specularColorMap;
	#endif
	#ifdef USE_SPECULAR_INTENSITYMAP
		uniform sampler2D specularIntensityMap;
	#endif
#endif
#ifdef USE_CLEARCOAT
	uniform float clearcoat;
	uniform float clearcoatRoughness;
#endif
#ifdef USE_DIFFUSE_ROUGHNESS
	uniform float diffuseRoughness;
	#ifdef USE_DIFFUSE_ROUGHNESSMAP
		uniform sampler2D diffuseRoughnessMap;
	#endif
#endif
#ifdef USE_DISPERSION
	uniform float dispersion;
#endif
#ifdef USE_RETROREFLECTION
	uniform float retroreflectivity;
#endif
#ifdef USE_IRIDESCENCE
	uniform float iridescence;
	uniform float iridescenceIOR;
	uniform float iridescenceThicknessMinimum;
	uniform float iridescenceThicknessMaximum;
#endif
#ifdef USE_SHEEN
	uniform vec3 sheenColor;
	uniform float sheenRoughness;
	#ifdef USE_SHEEN_COLORMAP
		uniform sampler2D sheenColorMap;
	#endif
	#ifdef USE_SHEEN_ROUGHNESSMAP
		uniform sampler2D sheenRoughnessMap;
	#endif
#endif
#ifdef USE_ANISOTROPY
	uniform vec2 anisotropyVector;
	#ifdef USE_ANISOTROPYMAP
		uniform sampler2D anisotropyMap;
	#endif
#endif
varying vec3 vViewPosition;
#include <common>
#include <dithering_pars_fragment>
#include <color_pars_fragment>
#include <uv_pars_fragment>
#include <map_pars_fragment>
#include <alphamap_pars_fragment>
#include <alphatest_pars_fragment>
#include <alphahash_pars_fragment>
#include <aomap_pars_fragment>
#include <lightmap_pars_fragment>
#include <emissivemap_pars_fragment>
#include <iridescence_fragment>
#include <envmap_common_pars_fragment>
#include <envmap_physical_pars_fragment>
#include <fog_pars_fragment>
#include <lights_pars_begin>
#include <normal_pars_fragment>
#include <lights_physical_pars_fragment>
#include <transmission_pars_fragment>
#include <shadowmap_pars_fragment>
#include <bumpmap_pars_fragment>
#include <normalmap_pars_fragment>
#include <clearcoat_pars_fragment>
#include <iridescence_pars_fragment>
#include <roughnessmap_pars_fragment>
#include <metalnessmap_pars_fragment>
#include <logdepthbuf_pars_fragment>
#include <clipping_planes_pars_fragment>
void main() {
	vec4 diffuseColor = vec4( diffuse, opacity );
	#include <clipping_planes_fragment>
	ReflectedLight reflectedLight = ReflectedLight( vec3( 0.0 ), vec3( 0.0 ), vec3( 0.0 ), vec3( 0.0 ) );
	vec3 totalEmissiveRadiance = emissive;
	#include <logdepthbuf_fragment>
	#include <map_fragment>
	#include <color_fragment>
	#include <alphamap_fragment>
	#include <alphatest_fragment>
	#include <alphahash_fragment>
	#include <roughnessmap_fragment>
	#include <metalnessmap_fragment>
	#include <normal_fragment_begin>
	#include <normal_fragment_maps>
	#include <clearcoat_normal_fragment_begin>
	#include <clearcoat_normal_fragment_maps>
	#include <emissivemap_fragment>
	#include <lights_physical_fragment>
	#include <lights_fragment_begin>
	#include <lights_fragment_maps>
	#include <lights_fragment_end>
	#include <aomap_fragment>
	vec3 totalDiffuse = reflectedLight.directDiffuse + reflectedLight.indirectDiffuse;
	vec3 totalSpecular = reflectedLight.directSpecular + reflectedLight.indirectSpecular;
	#include <transmission_fragment>
	vec3 outgoingLight = totalDiffuse + totalSpecular + totalEmissiveRadiance;
	#ifdef USE_SHEEN
 
		outgoingLight = outgoingLight + sheenSpecularDirect + sheenSpecularIndirect;
 
 	#endif
	#ifdef USE_CLEARCOAT
		float dotNVcc = saturate( dot( geometryClearcoatNormal, geometryViewDir ) );
		vec3 Fcc = F_Schlick( material.clearcoatF0, material.clearcoatF90, dotNVcc );
		outgoingLight = outgoingLight * ( 1.0 - material.clearcoat * Fcc ) + ( clearcoatSpecularDirect + clearcoatSpecularIndirect ) * material.clearcoat;
	#endif
	#include <opaque_fragment>
	#include <tonemapping_fragment>
	#include <colorspace_fragment>
	#include <fog_fragment>
	#include <premultiplied_alpha_fragment>
	#include <dithering_fragment>
}`,Uf=`#define TOON
varying vec3 vViewPosition;
#include <common>
#include <batching_pars_vertex>
#include <uv_pars_vertex>
#include <displacementmap_pars_vertex>
#include <color_pars_vertex>
#include <fog_pars_vertex>
#include <normal_pars_vertex>
#include <morphtarget_pars_vertex>
#include <skinning_pars_vertex>
#include <shadowmap_pars_vertex>
#include <logdepthbuf_pars_vertex>
#include <clipping_planes_pars_vertex>
void main() {
	#include <uv_vertex>
	#include <color_vertex>
	#include <morphinstance_vertex>
	#include <morphcolor_vertex>
	#include <batching_vertex>
	#include <beginnormal_vertex>
	#include <morphnormal_vertex>
	#include <skinbase_vertex>
	#include <skinnormal_vertex>
	#include <defaultnormal_vertex>
	#include <normal_vertex>
	#include <begin_vertex>
	#include <morphtarget_vertex>
	#include <skinning_vertex>
	#include <displacementmap_vertex>
	#include <project_vertex>
	#include <logdepthbuf_vertex>
	#include <clipping_planes_vertex>
	vViewPosition = - mvPosition.xyz;
	#include <worldpos_vertex>
	#include <shadowmap_vertex>
	#include <fog_vertex>
}`,Ff=`#define TOON
uniform vec3 diffuse;
uniform vec3 emissive;
uniform float opacity;
#include <common>
#include <dithering_pars_fragment>
#include <color_pars_fragment>
#include <uv_pars_fragment>
#include <map_pars_fragment>
#include <alphamap_pars_fragment>
#include <alphatest_pars_fragment>
#include <alphahash_pars_fragment>
#include <aomap_pars_fragment>
#include <lightmap_pars_fragment>
#include <emissivemap_pars_fragment>
#include <gradientmap_pars_fragment>
#include <fog_pars_fragment>
#include <bsdfs>
#include <lights_pars_begin>
#include <normal_pars_fragment>
#include <lights_toon_pars_fragment>
#include <shadowmap_pars_fragment>
#include <bumpmap_pars_fragment>
#include <normalmap_pars_fragment>
#include <logdepthbuf_pars_fragment>
#include <clipping_planes_pars_fragment>
void main() {
	vec4 diffuseColor = vec4( diffuse, opacity );
	#include <clipping_planes_fragment>
	ReflectedLight reflectedLight = ReflectedLight( vec3( 0.0 ), vec3( 0.0 ), vec3( 0.0 ), vec3( 0.0 ) );
	vec3 totalEmissiveRadiance = emissive;
	#include <logdepthbuf_fragment>
	#include <map_fragment>
	#include <color_fragment>
	#include <alphamap_fragment>
	#include <alphatest_fragment>
	#include <alphahash_fragment>
	#include <normal_fragment_begin>
	#include <normal_fragment_maps>
	#include <emissivemap_fragment>
	#include <lights_toon_fragment>
	#include <lights_fragment_begin>
	#include <lights_fragment_maps>
	#include <lights_fragment_end>
	#include <aomap_fragment>
	vec3 outgoingLight = reflectedLight.directDiffuse + reflectedLight.indirectDiffuse + totalEmissiveRadiance;
	#include <opaque_fragment>
	#include <tonemapping_fragment>
	#include <colorspace_fragment>
	#include <fog_fragment>
	#include <premultiplied_alpha_fragment>
	#include <dithering_fragment>
}`,Of=`uniform float size;
uniform float scale;
#include <common>
#include <color_pars_vertex>
#include <fog_pars_vertex>
#include <morphtarget_pars_vertex>
#include <logdepthbuf_pars_vertex>
#include <clipping_planes_pars_vertex>
#ifdef USE_POINTS_UV
	varying vec2 vUv;
	uniform mat3 uvTransform;
#endif
void main() {
	#ifdef USE_POINTS_UV
		vUv = ( uvTransform * vec3( uv, 1 ) ).xy;
	#endif
	#include <color_vertex>
	#include <morphinstance_vertex>
	#include <morphcolor_vertex>
	#include <begin_vertex>
	#include <morphtarget_vertex>
	#include <project_vertex>
	gl_PointSize = size;
	#ifdef USE_SIZEATTENUATION
		bool isPerspective = isPerspectiveMatrix( projectionMatrix );
		if ( isPerspective ) gl_PointSize *= ( scale / - mvPosition.z );
	#endif
	#include <logdepthbuf_vertex>
	#include <clipping_planes_vertex>
	#include <worldpos_vertex>
	#include <fog_vertex>
}`,Bf=`uniform vec3 diffuse;
uniform float opacity;
#include <common>
#include <color_pars_fragment>
#include <map_particle_pars_fragment>
#include <alphatest_pars_fragment>
#include <alphahash_pars_fragment>
#include <fog_pars_fragment>
#include <logdepthbuf_pars_fragment>
#include <clipping_planes_pars_fragment>
void main() {
	vec4 diffuseColor = vec4( diffuse, opacity );
	#include <clipping_planes_fragment>
	vec3 outgoingLight = vec3( 0.0 );
	#include <logdepthbuf_fragment>
	#include <map_particle_fragment>
	#include <color_fragment>
	#include <alphatest_fragment>
	#include <alphahash_fragment>
	outgoingLight = diffuseColor.rgb;
	#include <opaque_fragment>
	#include <tonemapping_fragment>
	#include <colorspace_fragment>
	#include <fog_fragment>
	#include <premultiplied_alpha_fragment>
}`,zf=`#include <common>
#include <batching_pars_vertex>
#include <fog_pars_vertex>
#include <morphtarget_pars_vertex>
#include <skinning_pars_vertex>
#include <logdepthbuf_pars_vertex>
#include <shadowmap_pars_vertex>
void main() {
	#include <batching_vertex>
	#include <beginnormal_vertex>
	#include <morphinstance_vertex>
	#include <morphnormal_vertex>
	#include <skinbase_vertex>
	#include <skinnormal_vertex>
	#include <defaultnormal_vertex>
	#include <begin_vertex>
	#include <morphtarget_vertex>
	#include <skinning_vertex>
	#include <project_vertex>
	#include <logdepthbuf_vertex>
	#include <worldpos_vertex>
	#include <shadowmap_vertex>
	#include <fog_vertex>
}`,kf=`uniform vec3 color;
uniform float opacity;
#include <common>
#include <fog_pars_fragment>
#include <bsdfs>
#include <lights_pars_begin>
#include <logdepthbuf_pars_fragment>
#include <shadowmap_pars_fragment>
#include <shadowmask_pars_fragment>
void main() {
	#include <logdepthbuf_fragment>
	gl_FragColor = vec4( color, opacity * ( 1.0 - getShadowMask() ) );
	#include <tonemapping_fragment>
	#include <colorspace_fragment>
	#include <fog_fragment>
	#include <premultiplied_alpha_fragment>
}`,Gf=`uniform float rotation;
uniform vec2 center;
#include <common>
#include <uv_pars_vertex>
#include <fog_pars_vertex>
#include <logdepthbuf_pars_vertex>
#include <clipping_planes_pars_vertex>
void main() {
	#include <uv_vertex>
	vec4 mvPosition = modelViewMatrix[ 3 ];
	vec2 scale = vec2( length( modelMatrix[ 0 ].xyz ), length( modelMatrix[ 1 ].xyz ) );
	#ifndef USE_SIZEATTENUATION
		bool isPerspective = isPerspectiveMatrix( projectionMatrix );
		if ( isPerspective ) scale *= - mvPosition.z;
	#endif
	vec2 alignedPosition = ( position.xy - ( center - vec2( 0.5 ) ) ) * scale;
	vec2 rotatedPosition;
	rotatedPosition.x = cos( rotation ) * alignedPosition.x - sin( rotation ) * alignedPosition.y;
	rotatedPosition.y = sin( rotation ) * alignedPosition.x + cos( rotation ) * alignedPosition.y;
	mvPosition.xy += rotatedPosition;
	gl_Position = projectionMatrix * mvPosition;
	#include <logdepthbuf_vertex>
	#include <clipping_planes_vertex>
	#include <fog_vertex>
}`,Hf=`uniform vec3 diffuse;
uniform float opacity;
#include <common>
#include <uv_pars_fragment>
#include <map_pars_fragment>
#include <alphamap_pars_fragment>
#include <alphatest_pars_fragment>
#include <alphahash_pars_fragment>
#include <fog_pars_fragment>
#include <logdepthbuf_pars_fragment>
#include <clipping_planes_pars_fragment>
void main() {
	vec4 diffuseColor = vec4( diffuse, opacity );
	#include <clipping_planes_fragment>
	vec3 outgoingLight = vec3( 0.0 );
	#include <logdepthbuf_fragment>
	#include <map_fragment>
	#include <alphamap_fragment>
	#include <alphatest_fragment>
	#include <alphahash_fragment>
	outgoingLight = diffuseColor.rgb;
	#include <opaque_fragment>
	#include <tonemapping_fragment>
	#include <colorspace_fragment>
	#include <fog_fragment>
}`,ne={alphahash_fragment:cu,alphahash_pars_fragment:hu,alphamap_fragment:uu,alphamap_pars_fragment:du,alphatest_fragment:fu,alphatest_pars_fragment:pu,aomap_fragment:mu,aomap_pars_fragment:gu,batching_pars_vertex:_u,batching_vertex:xu,begin_vertex:vu,beginnormal_vertex:yu,bsdfs:Mu,iridescence_fragment:Su,bumpmap_pars_fragment:bu,clipping_planes_fragment:wu,clipping_planes_pars_fragment:Au,clipping_planes_pars_vertex:Tu,clipping_planes_vertex:Eu,color_fragment:Ru,color_pars_fragment:Cu,color_pars_vertex:Iu,color_vertex:Pu,common:Lu,defaultnormal_vertex:Nu,displacementmap_pars_vertex:Du,displacementmap_vertex:Uu,emissivemap_fragment:Fu,emissivemap_pars_fragment:Ou,colorspace_fragment:Bu,colorspace_pars_fragment:zu,envmap_fragment:ku,envmap_common_pars_fragment:Gu,envmap_pars_fragment:Hu,envmap_pars_vertex:Wu,envmap_physical_pars_fragment:td,envmap_vertex:Vu,fog_vertex:Xu,fog_pars_vertex:qu,fog_fragment:Yu,fog_pars_fragment:Zu,gradientmap_pars_fragment:Ju,lightmap_pars_fragment:$u,lights_lambert_fragment:Ku,lights_lambert_pars_fragment:Qu,lights_pars_begin:ju,lights_toon_fragment:ed,lights_toon_pars_fragment:nd,lights_phong_fragment:id,lights_phong_pars_fragment:sd,lights_physical_fragment:rd,lights_physical_pars_fragment:ad,lights_fragment_begin:od,lights_fragment_maps:ld,lights_fragment_end:cd,lightprobes_pars_fragment:hd,logdepthbuf_fragment:ud,logdepthbuf_pars_fragment:dd,logdepthbuf_pars_vertex:fd,logdepthbuf_vertex:pd,map_fragment:md,map_pars_fragment:gd,map_particle_fragment:_d,map_particle_pars_fragment:xd,metalnessmap_fragment:vd,metalnessmap_pars_fragment:yd,morphinstance_vertex:Md,morphcolor_vertex:Sd,morphnormal_vertex:bd,morphtarget_pars_vertex:wd,morphtarget_vertex:Ad,normal_fragment_begin:Td,normal_fragment_maps:Ed,normal_pars_fragment:Rd,normal_pars_vertex:Cd,normal_vertex:Id,normalmap_pars_fragment:Pd,clearcoat_normal_fragment_begin:Ld,clearcoat_normal_fragment_maps:Nd,clearcoat_pars_fragment:Dd,iridescence_pars_fragment:Ud,opaque_fragment:Fd,packing:Od,premultiplied_alpha_fragment:Bd,project_vertex:zd,dithering_fragment:kd,dithering_pars_fragment:Gd,roughnessmap_fragment:Hd,roughnessmap_pars_fragment:Wd,shadowmap_pars_fragment:Vd,shadowmap_pars_vertex:Xd,shadowmap_vertex:qd,shadowmask_pars_fragment:Yd,skinbase_vertex:Zd,skinning_pars_vertex:Jd,skinning_vertex:$d,skinnormal_vertex:Kd,specularmap_fragment:Qd,specularmap_pars_fragment:jd,tonemapping_fragment:tf,tonemapping_pars_fragment:ef,transmission_fragment:nf,transmission_pars_fragment:sf,uv_pars_fragment:rf,uv_pars_vertex:af,uv_vertex:of,worldpos_vertex:lf,background_vert:cf,background_frag:hf,backgroundCube_vert:uf,backgroundCube_frag:df,cube_vert:ff,cube_frag:pf,depth_vert:mf,depth_frag:gf,distance_vert:_f,distance_frag:xf,equirect_vert:vf,equirect_frag:yf,linedashed_vert:Mf,linedashed_frag:Sf,meshbasic_vert:bf,meshbasic_frag:wf,meshlambert_vert:Af,meshlambert_frag:Tf,meshmatcap_vert:Ef,meshmatcap_frag:Rf,meshnormal_vert:Cf,meshnormal_frag:If,meshphong_vert:Pf,meshphong_frag:Lf,meshphysical_vert:Nf,meshphysical_frag:Df,meshtoon_vert:Uf,meshtoon_frag:Ff,points_vert:Of,points_frag:Bf,shadow_vert:zf,shadow_frag:kf,sprite_vert:Gf,sprite_frag:Hf},Rt={common:{diffuse:{value:new le(16777215)},opacity:{value:1},map:{value:null},mapTransform:{value:new Zt},alphaMap:{value:null},alphaMapTransform:{value:new Zt},alphaTest:{value:0}},specularmap:{specularMap:{value:null},specularMapTransform:{value:new Zt}},envmap:{envMap:{value:null},envMapRotation:{value:new Zt},reflectivity:{value:1},ior:{value:1.5},refractionRatio:{value:0.98},dfgLUT:{value:null}},aomap:{aoMap:{value:null},aoMapIntensity:{value:1},aoMapTransform:{value:new Zt}},lightmap:{lightMap:{value:null},lightMapIntensity:{value:1},lightMapTransform:{value:new Zt}},bumpmap:{bumpMap:{value:null},bumpMapTransform:{value:new Zt},bumpScale:{value:1}},normalmap:{normalMap:{value:null},normalMapTransform:{value:new Zt},normalScale:{value:new fe(1,1)}},displacementmap:{displacementMap:{value:null},displacementMapTransform:{value:new Zt},displacementScale:{value:1},displacementBias:{value:0}},emissivemap:{emissiveMap:{value:null},emissiveMapTransform:{value:new Zt}},metalnessmap:{metalnessMap:{value:null},metalnessMapTransform:{value:new Zt}},roughnessmap:{roughnessMap:{value:null},roughnessMapTransform:{value:new Zt}},gradientmap:{gradientMap:{value:null}},fog:{fogDensity:{value:0.00025},fogNear:{value:1},fogFar:{value:2000},fogColor:{value:new le(16777215)}},lights:{ambientLightColor:{value:[]},lightProbe:{value:[]},sunLights:{value:[],properties:{direction:{},color:{}}},sunLightShadows:{value:[],properties:{shadowIntensity:1,shadowBias:{},shadowNormalBias:{},shadowRadius:{},shadowMapSize:{}}},sunShadowMatrix:{value:[]},sunShadowCascade:{value:[]},directionalLights:{value:[],properties:{direction:{},color:{}}},directionalLightShadows:{value:[],properties:{shadowIntensity:1,shadowBias:{},shadowNormalBias:{},shadowRadius:{},shadowMapSize:{}}},directionalShadowMatrix:{value:[]},spotLights:{value:[],properties:{color:{},position:{},direction:{},distance:{},coneCos:{},penumbraCos:{},decay:{}}},spotLightShadows:{value:[],properties:{shadowIntensity:1,shadowBias:{},shadowNormalBias:{},shadowRadius:{},shadowMapSize:{}}},spotLightMap:{value:[]},spotLightMatrix:{value:[]},pointLights:{value:[],properties:{color:{},position:{},decay:{},distance:{}}},pointLightShadows:{value:[],properties:{shadowIntensity:1,shadowBias:{},shadowNormalBias:{},shadowRadius:{},shadowMapSize:{},shadowCameraNear:{},shadowCameraFar:{}}},pointShadowMatrix:{value:[]},hemisphereLights:{value:[],properties:{direction:{},skyColor:{},groundColor:{}}},rectAreaLights:{value:[],properties:{color:{},position:{},width:{},height:{}}},ltc_1:{value:null},ltc_2:{value:null},probesSH:{value:[]},probesMin:{value:[]},probesMax:{value:[]},probesResolution:{value:[]}},points:{diffuse:{value:new le(16777215)},opacity:{value:1},size:{value:1},scale:{value:1},map:{value:null},alphaMap:{value:null},alphaMapTransform:{value:new Zt},alphaTest:{value:0},uvTransform:{value:new Zt}},sprite:{diffuse:{value:new le(16777215)},opacity:{value:1},center:{value:new fe(0.5,0.5)},rotation:{value:0},map:{value:null},mapTransform:{value:new Zt},alphaMap:{value:null},alphaMapTransform:{value:new Zt},alphaTest:{value:0}}},An={basic:{uniforms:qe([Rt.common,Rt.specularmap,Rt.envmap,Rt.aomap,Rt.lightmap,Rt.fog]),vertexShader:ne.meshbasic_vert,fragmentShader:ne.meshbasic_frag},lambert:{uniforms:qe([Rt.common,Rt.specularmap,Rt.envmap,Rt.aomap,Rt.lightmap,Rt.emissivemap,Rt.bumpmap,Rt.normalmap,Rt.displacementmap,Rt.fog,Rt.lights,{emissive:{value:new le(0)},envMapIntensity:{value:1}}]),vertexShader:ne.meshlambert_vert,fragmentShader:ne.meshlambert_frag},phong:{uniforms:qe([Rt.common,Rt.specularmap,Rt.envmap,Rt.aomap,Rt.lightmap,Rt.emissivemap,Rt.bumpmap,Rt.normalmap,Rt.displacementmap,Rt.fog,Rt.lights,{emissive:{value:new le(0)},specular:{value:new le(1118481)},shininess:{value:30},envMapIntensity:{value:1}}]),vertexShader:ne.meshphong_vert,fragmentShader:ne.meshphong_frag},standard:{uniforms:qe([Rt.common,Rt.envmap,Rt.aomap,Rt.lightmap,Rt.emissivemap,Rt.bumpmap,Rt.normalmap,Rt.displacementmap,Rt.roughnessmap,Rt.metalnessmap,Rt.fog,Rt.lights,{emissive:{value:new le(0)},roughness:{value:1},metalness:{value:0},envMapIntensity:{value:1}}]),vertexShader:ne.meshphysical_vert,fragmentShader:ne.meshphysical_frag},toon:{uniforms:qe([Rt.common,Rt.aomap,Rt.lightmap,Rt.emissivemap,Rt.bumpmap,Rt.normalmap,Rt.displacementmap,Rt.gradientmap,Rt.fog,Rt.lights,{emissive:{value:new le(0)}}]),vertexShader:ne.meshtoon_vert,fragmentShader:ne.meshtoon_frag},matcap:{uniforms:qe([Rt.common,Rt.bumpmap,Rt.normalmap,Rt.displacementmap,Rt.fog,{matcap:{value:null}}]),vertexShader:ne.meshmatcap_vert,fragmentShader:ne.meshmatcap_frag},points:{uniforms:qe([Rt.points,Rt.fog]),vertexShader:ne.points_vert,fragmentShader:ne.points_frag},dashed:{uniforms:qe([Rt.common,Rt.fog,{scale:{value:1},dashSize:{value:1},totalSize:{value:2}}]),vertexShader:ne.linedashed_vert,fragmentShader:ne.linedashed_frag},depth:{uniforms:qe([Rt.common,Rt.displacementmap]),vertexShader:ne.depth_vert,fragmentShader:ne.depth_frag},normal:{uniforms:qe([Rt.common,Rt.bumpmap,Rt.normalmap,Rt.displacementmap,{opacity:{value:1}}]),vertexShader:ne.meshnormal_vert,fragmentShader:ne.meshnormal_frag},sprite:{uniforms:qe([Rt.sprite,Rt.fog]),vertexShader:ne.sprite_vert,fragmentShader:ne.sprite_frag},background:{uniforms:{uvTransform:{value:new Zt},t2D:{value:null},backgroundIntensity:{value:1}},vertexShader:ne.background_vert,fragmentShader:ne.background_frag},backgroundCube:{uniforms:{envMap:{value:null},backgroundBlurriness:{value:0},backgroundIntensity:{value:1},backgroundRotation:{value:new Zt}},vertexShader:ne.backgroundCube_vert,fragmentShader:ne.backgroundCube_frag},cube:{uniforms:{tCube:{value:null},tFlip:{value:-1},opacity:{value:1}},vertexShader:ne.cube_vert,fragmentShader:ne.cube_frag},equirect:{uniforms:{tEquirect:{value:null}},vertexShader:ne.equirect_vert,fragmentShader:ne.equirect_frag},distance:{uniforms:qe([Rt.common,Rt.displacementmap,{referencePosition:{value:new W},nearDistance:{value:1},farDistance:{value:1000}}]),vertexShader:ne.distance_vert,fragmentShader:ne.distance_frag},shadow:{uniforms:qe([Rt.lights,Rt.fog,{color:{value:new le(0)},opacity:{value:1}}]),vertexShader:ne.shadow_vert,fragmentShader:ne.shadow_frag}};An.physical={uniforms:qe([An.standard.uniforms,{clearcoat:{value:0},clearcoatMap:{value:null},clearcoatMapTransform:{value:new Zt},clearcoatNormalMap:{value:null},clearcoatNormalMapTransform:{value:new Zt},clearcoatNormalScale:{value:new fe(1,1)},clearcoatRoughness:{value:0},clearcoatRoughnessMap:{value:null},clearcoatRoughnessMapTransform:{value:new Zt},diffuseRoughness:{value:0},diffuseRoughnessMap:{value:null},diffuseRoughnessMapTransform:{value:new Zt},dispersion:{value:0},retroreflectivity:{value:0},iridescence:{value:0},iridescenceMap:{value:null},iridescenceMapTransform:{value:new Zt},iridescenceIOR:{value:1.3},iridescenceThicknessMinimum:{value:100},iridescenceThicknessMaximum:{value:400},iridescenceThicknessMap:{value:null},iridescenceThicknessMapTransform:{value:new Zt},sheen:{value:0},sheenColor:{value:new le(0)},sheenColorMap:{value:null},sheenColorMapTransform:{value:new Zt},sheenRoughness:{value:1},sheenRoughnessMap:{value:null},sheenRoughnessMapTransform:{value:new Zt},transmission:{value:0},transmissionMap:{value:null},transmissionMapTransform:{value:new Zt},transmissionSamplerSize:{value:new fe},transmissionSamplerMap:{value:null},thickness:{value:0},thicknessMap:{value:null},thicknessMapTransform:{value:new Zt},attenuationDistance:{value:0},attenuationColor:{value:new le(0)},specularColor:{value:new le(1,1,1)},specularColorMap:{value:null},specularColorMapTransform:{value:new Zt},specularIntensity:{value:1},specularIntensityMap:{value:null},specularIntensityMapTransform:{value:new Zt},anisotropyVector:{value:new fe},anisotropyMap:{value:null},anisotropyMapTransform:{value:new Zt}}]),vertexShader:ne.meshphysical_vert,fragmentShader:ne.meshphysical_frag};var Sr={r:0,b:0,g:0},Wf=new Se,Kc=new Zt;Kc.set(-1,0,0,0,1,0,0,0,1);function Vf(t,e,n,i,s,r){let a=new le(0),o=s===!0?0:1,l,c,p=null,h=0,u=null;function d(M){let T=M.isScene===!0?M.background:null;if(T&&T.isTexture){let S=M.backgroundBlurriness>0;T=e.get(T,S)}return T}function v(M){let T=!1,S=d(M);if(S===null)f(a,o);else if(S&&S.isColor)f(S,1),T=!0;let E=t.xr.getEnvironmentBlendMode();if(E==="additive")n.buffers.color.setClear(0,0,0,1,r);else if(E==="alpha-blend")n.buffers.color.setClear(0,0,0,0,r);if(t.autoClear||T)n.buffers.depth.setTest(!0),n.buffers.depth.setMask(!0),n.buffers.color.setMask(!0),t.clear(t.autoClearColor,t.autoClearDepth,t.autoClearStencil)}function w(M,T){let S=d(T);if(S&&S.isCubeTexture){if(c===void 0)c=new hn(new Wi(1,1,1),new vn({name:"BackgroundCubeMaterial",uniforms:fi(An.backgroundCube.uniforms),vertexShader:An.backgroundCube.vertexShader,fragmentShader:An.backgroundCube.fragmentShader,side:Qe,depthTest:!1,depthWrite:!1,fog:!1,allowOverride:!1})),c.geometry.deleteAttribute("normal"),c.geometry.deleteAttribute("uv"),c.onBeforeRender=function(E,P,g){this.matrixWorld.copyPosition(g.matrixWorld)},Object.defineProperty(c.material,"envMap",{get:function(){return this.uniforms.envMap.value}}),i.update(c);if(c.material.uniforms.envMap.value=S,c.material.uniforms.backgroundBlurriness.value=T.backgroundBlurriness,c.material.uniforms.backgroundIntensity.value=T.backgroundIntensity,c.material.uniforms.backgroundRotation.value.setFromMatrix4(Wf.makeRotationFromEuler(T.backgroundRotation)).transpose(),S.isRenderTargetTexture===!1)c.material.uniforms.backgroundRotation.value.premultiply(Kc);if(c.material.toneMapped=ae.getTransfer(S.colorSpace)!==ye,p!==S||h!==S.version||u!==t.toneMapping)c.material.needsUpdate=!0,p=S,h=S.version,u=t.toneMapping;c.layers.enableAll(),M.unshift(c,c.geometry,c.material,0,0,null)}else if(S&&S.isTexture){if(l===void 0)l=new hn(new ds(2,2),new vn({name:"BackgroundMaterial",uniforms:fi(An.background.uniforms),vertexShader:An.background.vertexShader,fragmentShader:An.background.fragmentShader,side:Bi,depthTest:!1,depthWrite:!1,fog:!1,allowOverride:!1})),l.geometry.deleteAttribute("normal"),Object.defineProperty(l.material,"map",{get:function(){return this.uniforms.t2D.value}}),i.update(l);if(l.material.uniforms.t2D.value=S,l.material.uniforms.backgroundIntensity.value=T.backgroundIntensity,l.material.toneMapped=ae.getTransfer(S.colorSpace)!==ye,S.matrixAutoUpdate===!0)S.updateMatrix();if(l.material.uniforms.uvTransform.value.copy(S.matrix),p!==S||h!==S.version||u!==t.toneMapping)l.material.needsUpdate=!0,p=S,h=S.version,u=t.toneMapping;l.layers.enableAll(),M.unshift(l,l.geometry,l.material,0,0,null)}}function f(M,T){M.getRGB(Sr,_o(t)),n.buffers.color.setClear(Sr.r,Sr.g,Sr.b,T,r)}function m(){if(c!==void 0)c.geometry.dispose(),c.material.dispose(),c=void 0;if(l!==void 0)l.geometry.dispose(),l.material.dispose(),l=void 0}return{getClearColor:function(){return a},setClearColor:function(M,T=1){a.set(M),o=T,f(a,o)},getClearAlpha:function(){return o},setClearAlpha:function(M){o=M,f(a,o)},render:v,addToRenderList:w,dispose:m}}function Xf(t,e){let n=t.getParameter(t.MAX_VERTEX_ATTRIBS),i={},s=u(null),r=s,a=!1;function o(N,X,rt,R,j){let tt=!1,G=h(N,R,rt,X);if(r!==G)r=G,c(r.object);if(tt=d(N,R,rt,j),tt)v(N,R,rt,j);if(j!==null)e.update(j,t.ELEMENT_ARRAY_BUFFER);if(tt||a){if(a=!1,S(N,X,rt,R),j!==null)t.bindBuffer(t.ELEMENT_ARRAY_BUFFER,e.get(j).buffer)}}function l(){return t.createVertexArray()}function c(N){return t.bindVertexArray(N)}function p(N){return t.deleteVertexArray(N)}function h(N,X,rt,R){let j=R.wireframe===!0,tt=i[X.id];if(tt===void 0)tt={},i[X.id]=tt;let G=N.isInstancedMesh===!0?N.id:0,pt=tt[G];if(pt===void 0)pt={},tt[G]=pt;let J=pt[rt.id];if(J===void 0)J={},pt[rt.id]=J;let st=J[j];if(st===void 0)st=u(l()),J[j]=st;return st}function u(N){let X=[],rt=[],R=[];for(let j=0;j<n;j++)X[j]=0,rt[j]=0,R[j]=0;return{geometry:null,program:null,wireframe:!1,newAttributes:X,enabledAttributes:rt,attributeDivisors:R,object:N,attributes:{},index:null}}function d(N,X,rt,R){let j=r.attributes,tt=X.attributes,G=0,pt=rt.getAttributes();for(let J in pt)if(pt[J].location>=0){let k=j[J],It=tt[J];if(It===void 0){if(J==="instanceMatrix"&&N.instanceMatrix)It=N.instanceMatrix;if(J==="instanceColor"&&N.instanceColor)It=N.instanceColor}if(k===void 0)return!0;if(k.attribute!==It)return!0;if(It&&k.data!==It.data)return!0;G++}if(r.attributesNum!==G)return!0;if(r.index!==R)return!0;return!1}function v(N,X,rt,R){let j={},tt=X.attributes,G=0,pt=rt.getAttributes();for(let J in pt)if(pt[J].location>=0){let k=tt[J];if(k===void 0){if(J==="instanceMatrix"&&N.instanceMatrix)k=N.instanceMatrix;if(J==="instanceColor"&&N.instanceColor)k=N.instanceColor}let It={};if(It.attribute=k,k&&k.data)It.data=k.data;j[J]=It,G++}r.attributes=j,r.attributesNum=G,r.index=R}function w(){let N=r.newAttributes;for(let X=0,rt=N.length;X<rt;X++)N[X]=0}function f(N){m(N,0)}function m(N,X){let rt=r.newAttributes,R=r.enabledAttributes,j=r.attributeDivisors;if(rt[N]=1,R[N]===0)t.enableVertexAttribArray(N),R[N]=1;if(j[N]!==X)t.vertexAttribDivisor(N,X),j[N]=X}function M(){let N=r.newAttributes,X=r.enabledAttributes;for(let rt=0,R=X.length;rt<R;rt++)if(X[rt]!==N[rt])t.disableVertexAttribArray(rt),X[rt]=0}function T(N,X,rt,R,j,tt,G){if(G===!0)t.vertexAttribIPointer(N,X,rt,j,tt);else t.vertexAttribPointer(N,X,rt,R,j,tt)}function S(N,X,rt,R){w();let j=R.attributes,tt=rt.getAttributes(),G=X.defaultAttributeValues;for(let pt in tt){let J=tt[pt];if(J.location>=0){let st=j[pt];if(st===void 0){if(pt==="instanceMatrix"&&N.instanceMatrix)st=N.instanceMatrix;if(pt==="instanceColor"&&N.instanceColor)st=N.instanceColor}if(st!==void 0){let k=st.normalized,It=st.itemSize,nt=e.get(st);if(nt===void 0)continue;let{buffer:ct,type:yt,bytesPerElement:Gt}=nt,zt=yt===t.INT||yt===t.UNSIGNED_INT||st.gpuType===ga;if(st.isInterleavedBufferAttribute){let Dt=st.data,ue=Dt.isInterleavedBuffer===!0?Dt.stride:Dt.itemSize,ee=st.offset;if(Dt.isInstancedInterleavedBuffer||Dt.isInstancedBufferAttribute){for(let Jt=0;Jt<J.locationSize;Jt++)m(J.location+Jt,Dt.meshPerAttribute);if(N.isInstancedMesh!==!0&&R._maxInstanceCount===void 0)R._maxInstanceCount=Dt.meshPerAttribute*Dt.count}else for(let Jt=0;Jt<J.locationSize;Jt++)f(J.location+Jt);t.bindBuffer(t.ARRAY_BUFFER,ct);for(let Jt=0;Jt<J.locationSize;Jt++)T(J.location+Jt,It/J.locationSize,yt,k,ue*Gt,(ee+It/J.locationSize*Jt)*Gt,zt)}else{if(st.isInstancedBufferAttribute){for(let Dt=0;Dt<J.locationSize;Dt++)m(J.location+Dt,st.meshPerAttribute);if(N.isInstancedMesh!==!0&&R._maxInstanceCount===void 0)R._maxInstanceCount=st.meshPerAttribute*st.count}else for(let Dt=0;Dt<J.locationSize;Dt++)f(J.location+Dt);t.bindBuffer(t.ARRAY_BUFFER,ct);for(let Dt=0;Dt<J.locationSize;Dt++)T(J.location+Dt,It/J.locationSize,yt,k,It*Gt,It/J.locationSize*Dt*Gt,zt)}}else if(G!==void 0){let k=G[pt];if(k!==void 0)switch(k.length){case 2:t.vertexAttrib2fv(J.location,k);break;case 3:t.vertexAttrib3fv(J.location,k);break;case 4:t.vertexAttrib4fv(J.location,k);break;default:t.vertexAttrib1fv(J.location,k)}}}}M()}function E(){D();for(let N in i){let X=i[N];for(let rt in X){let R=X[rt];for(let j in R){let tt=R[j];for(let G in tt)p(tt[G].object),delete tt[G];delete R[j]}}delete i[N]}}function P(N){if(i[N.id]===void 0)return;let X=i[N.id];for(let rt in X){let R=X[rt];for(let j in R){let tt=R[j];for(let G in tt)p(tt[G].object),delete tt[G];delete R[j]}}delete i[N.id]}function g(N){for(let X in i){let rt=i[X];for(let R in rt){let j=rt[R];if(j[N.id]===void 0)continue;let tt=j[N.id];for(let G in tt)p(tt[G].object),delete tt[G];delete j[N.id]}}}function y(N){for(let X in i){let rt=i[X],R=N.isInstancedMesh===!0?N.id:0,j=rt[R];if(j===void 0)continue;for(let tt in j){let G=j[tt];for(let pt in G)p(G[pt].object),delete G[pt];delete j[tt]}if(delete rt[R],Object.keys(rt).length===0)delete i[X]}}function D(){if(q(),a=!0,r===s)return;r=s,c(r.object)}function q(){s.geometry=null,s.program=null,s.wireframe=!1}return{setup:o,reset:D,resetDefaultState:q,dispose:E,releaseStatesOfGeometry:P,releaseStatesOfObject:y,releaseStatesOfProgram:g,initAttributes:w,enableAttribute:f,disableUnusedAttributes:M}}function qf(t,e,n){let i;function s(l){i=l}function r(l,c){t.drawArrays(i,l,c),n.update(c,i,1)}function a(l,c,p){if(p===0)return;t.drawArraysInstanced(i,l,c,p),n.update(c,i,p)}function o(l,c,p){if(p===0)return;e.get("WEBGL_multi_draw").multiDrawArraysWEBGL(i,l,0,c,0,p);let u=0;for(let d=0;d<p;d++)u+=c[d];n.update(u,i,1)}this.setMode=s,this.render=r,this.renderInstances=a,this.renderMultiDraw=o}function Yf(t,e,n,i){let s;function r(){if(s!==void 0)return s;if(e.has("EXT_texture_filter_anisotropic")===!0){let g=e.get("EXT_texture_filter_anisotropic");s=t.getParameter(g.MAX_TEXTURE_MAX_ANISOTROPY_EXT)}else s=0;return s}function a(g){if(g!==Fn&&i.convert(g)!==t.getParameter(t.IMPLEMENTATION_COLOR_READ_FORMAT))return!1;return!0}function o(g){let y=g===bn&&(e.has("EXT_color_buffer_half_float")||e.has("EXT_color_buffer_float"));if(g!==xn&&g!==Un&&!y&&i.convert(g)!==t.getParameter(t.IMPLEMENTATION_COLOR_READ_TYPE))return!1;return!0}function l(g){if(g==="highp"){if(t.getShaderPrecisionFormat(t.VERTEX_SHADER,t.HIGH_FLOAT).precision>0&&t.getShaderPrecisionFormat(t.FRAGMENT_SHADER,t.HIGH_FLOAT).precision>0)return"highp";g="mediump"}if(g==="mediump"){if(t.getShaderPrecisionFormat(t.VERTEX_SHADER,t.MEDIUM_FLOAT).precision>0&&t.getShaderPrecisionFormat(t.FRAGMENT_SHADER,t.MEDIUM_FLOAT).precision>0)return"mediump"}return"lowp"}let c=n.precision!==void 0?n.precision:"highp",p=l(c);if(p!==c)Wt("WebGLRenderer:",c,"not supported, using",p,"instead."),c=p;let h=n.reversedDepthBuffer===!0&&e.has("EXT_clip_control"),u=n.logarithmicDepthBuffer===!0&&h===!1;if(n.reversedDepthBuffer===!0&&h===!1)Wt("WebGLRenderer: Unable to use reversed depth buffer due to missing EXT_clip_control extension. Fallback to default depth buffer.");if(n.logarithmicDepthBuffer===!0&&h===!0)Wt("WebGLRenderer: Logarithmic depth buffer is ignored when reversed depth buffer is enabled.");let d=t.getParameter(t.MAX_TEXTURE_IMAGE_UNITS),v=t.getParameter(t.MAX_VERTEX_TEXTURE_IMAGE_UNITS),w=t.getParameter(t.MAX_TEXTURE_SIZE),f=t.getParameter(t.MAX_CUBE_MAP_TEXTURE_SIZE),m=t.getParameter(t.MAX_VERTEX_ATTRIBS),M=t.getParameter(t.MAX_VERTEX_UNIFORM_VECTORS),T=t.getParameter(t.MAX_VARYING_VECTORS),S=t.getParameter(t.MAX_FRAGMENT_UNIFORM_VECTORS),E=t.getParameter(t.MAX_SAMPLES),P=t.getParameter(t.SAMPLES);return{isWebGL2:!0,getMaxAnisotropy:r,getMaxPrecision:l,textureFormatReadable:a,textureTypeReadable:o,precision:c,logarithmicDepthBuffer:u,reversedDepthBuffer:h,maxTextures:d,maxVertexTextures:v,maxTextureSize:w,maxCubemapSize:f,maxAttributes:m,maxVertexUniforms:M,maxVaryings:T,maxFragmentUniforms:S,maxSamples:E,samples:P}}function Zf(t){let e=this,n=null,i=0,s=!1,r=!1,a=new Mn,o=new Zt,l={value:null,needsUpdate:!1};this.uniform=l,this.numPlanes=0,this.numIntersection=0,this.init=function(h,u){let d=h.length!==0||u||i!==0||s;return s=u,i=h.length,d},this.beginShadows=function(){r=!0,p(null)},this.endShadows=function(){r=!1},this.setGlobalState=function(h,u){n=p(h,u,0)},this.setState=function(h,u,d){let{clippingPlanes:v,clipIntersection:w,clipShadows:f}=h,m=t.get(h);if(!s||v===null||v.length===0||r&&!f)if(r)p(null);else c();else{let M=r?0:i,T=M*4,S=m.clippingState||null;l.value=S,S=p(v,u,T,d);for(let E=0;E!==T;++E)S[E]=n[E];m.clippingState=S,this.numIntersection=w?this.numPlanes:0,this.numPlanes+=M}};function c(){if(l.value!==n)l.value=n,l.needsUpdate=i>0;e.numPlanes=i,e.numIntersection=0}function p(h,u,d,v){let w=h!==null?h.length:0,f=null;if(w!==0){if(f=l.value,v!==!0||f===null){let m=d+w*4,M=u.matrixWorldInverse;if(o.getNormalMatrix(M),f===null||f.length<m)f=new Float32Array(m);for(let T=0,S=d;T!==w;++T,S+=4)a.copy(h[T]).applyMatrix4(M,o),a.normal.toArray(f,S),f[S+3]=a.constant}l.value=f,l.needsUpdate=!0}return e.numPlanes=w,e.numIntersection=0,f}}class Ar extends cn{constructor(t=1,e={}){super(t,t,e);this.isWebGLCubeRenderTarget=!0;let n={width:t,height:t,depth:1},i=[n,n,n,n,n,n];this.texture=new _r(i),this._setTextureOptions(e),this.texture.isRenderTargetTexture=!0}fromEquirectangularTexture(t,e){this.texture.type=e.type,this.texture.colorSpace=e.colorSpace,this.texture.generateMipmaps=e.generateMipmaps,this.texture.minFilter=e.minFilter,this.texture.magFilter=e.magFilter;let n={uniforms:{tEquirect:{value:null}},vertexShader:`

				varying vec3 vWorldDirection;

				vec3 transformDirection( in vec3 dir, in mat4 matrix ) {

					return normalize( ( matrix * vec4( dir, 0.0 ) ).xyz );

				}

				void main() {

					vWorldDirection = transformDirection( position, modelMatrix );

					#include <begin_vertex>
					#include <project_vertex>

				}
			`,fragmentShader:`

				uniform sampler2D tEquirect;

				varying vec3 vWorldDirection;

				#include <common>

				void main() {

					vec3 direction = normalize( vWorldDirection );

					vec2 sampleUV = equirectUv( direction );

					gl_FragColor = texture2D( tEquirect, sampleUV );

				}
			`},i=new Wi(5,5,5),s=new vn({name:"CubemapFromEquirect",uniforms:fi(n.uniforms),vertexShader:n.vertexShader,fragmentShader:n.fragmentShader,side:Qe,blending:ai});s.uniforms.tEquirect.value=e;let r=new hn(i,s),a=e.minFilter;if(e.minFilter===Yn)e.minFilter=ln;return new ps(1,10,this).update(t,r),e.minFilter=a,r.geometry.dispose(),r.material.dispose(),this}clear(t,e=!0,n=!0,i=!0){let s=t.getRenderTarget();for(let r=0;r<6;r++)t.setRenderTarget(this,r),t.clear(e,n,i);t.setRenderTarget(s)}}var Cc=256,Jf=3,$f=20,Qc=256,jc=16,Kf=3,Qf=new W,jf=new le;class wr{constructor(t){this._renderer=t,this._cubeSize=0,this._sourceTarget=null,this._cubeCamera=new ps(1,10,null),this._boxMesh=new hn(new Wi(5,5,5),null),this._cubemapMaterial=null,this._equirectMaterial=null,this._blurMaterial=null,this._ggxMaterial=null,this._integrationMaterial=null}fromScene(t,e=0,n=0.1,i=100,s={}){let{size:r=256,position:a=Qf}=s,o=this._renderer;this._setSize(r);let l=this._allocateTarget(),c=this._getSourceTarget(!0);if(e>0)c.texture.mipmapsAutoUpdate=!1;let{autoClear:p,autoClearColor:h,autoClearDepth:u,autoClearStencil:d}=o,v=t.background;if(o.autoClear=!0,o.autoClearColor=!0,o.autoClearDepth=!0,o.autoClearStencil=!0,v===null)t.background=o.getClearColor(jf);let w=new ps(n,i,c);if(w.position.copy(a),w.update(o,t),o.autoClear=p,o.autoClearColor=h,o.autoClearDepth=u,o.autoClearStencil=d,t.background=v,e>0)c.texture.mipmapsAutoUpdate=!0,this._blur(l,e);return this._applyPMREM(l),l}fromEquirectangular(t,e=null){return this._fromTexture(t,e)}fromCubemap(t,e=null){return this._fromTexture(t,e)}compileCubemapShader(){if(this._cubemapMaterial===null)this._cubemapMaterial=Lc(),this._compileMaterial(this._cubemapMaterial)}compileEquirectangularShader(){if(this._equirectMaterial===null)this._equirectMaterial=Pc(),this._compileMaterial(this._equirectMaterial)}dispose(){if(this._sourceTarget!==null)this._sourceTarget.dispose();if(this._cubemapMaterial!==null)this._cubemapMaterial.dispose();if(this._equirectMaterial!==null)this._equirectMaterial.dispose();if(this._blurMaterial!==null)this._blurMaterial.dispose();if(this._ggxMaterial!==null)this._ggxMaterial.dispose();if(this._integrationMaterial!==null)this._integrationMaterial.dispose();this._boxMesh.geometry.dispose()}static lodToRoughness(t,e){return e>0?1-Math.sqrt(1-t/e):0}_setSize(t){this._cubeSize=Math.max(Cc,Sc(t))}_fromTexture(t,e){if(t.mapping===rs||t.mapping===ki)this._setSize(t.image.length===0?Cc:t.image[0].width||t.image[0].image.width);else this._setSize(t.image.width/4);let n=e||this._allocateTarget();return this._textureToCubemap(t),this._applyPMREM(n),n}_allocateTarget(){let t=this._cubeSize,e=Ic(t,!1,!1),n=Math.log2(t)-Jf;for(let i=0;i<=n;i++)e.texture.mipmaps.push({width:t>>i,height:t>>i});return e.texture.name="PMREM",e.texture.isPMREMTexture=!0,e}_getSourceTarget(t=!1){let e=this._cubeSize,n=this._sourceTarget;if(n===null||n.width!==e||t&&n.depthBuffer===!1){if(n!==null)n.dispose();this._sourceTarget=Ic(e,!0,t)}return this._sourceTarget}_compileMaterial(t){this._boxMesh.material=t,this._renderer.compile(this._boxMesh,this._cubeCamera.children[0])}_renderCube(t,e,n){let i=this._boxMesh,s=this._cubeCamera;i.material=n,s.renderTarget=t,s.activeMipmapLevel=e;let r=t.width>>e;t.viewport.set(0,0,r,r),s.update(this._renderer,i),t.viewport.set(0,0,t.width,t.height)}_textureToCubemap(t){let e;if(t.mapping===rs||t.mapping===ki){if(this._cubemapMaterial===null)this._cubemapMaterial=Lc();e=this._cubemapMaterial,e.uniforms.flipEnvMap.value=t.isRenderTargetTexture===!1?-1:1}else{if(this._equirectMaterial===null)this._equirectMaterial=Pc();e=this._equirectMaterial}e.uniforms.envMap.value=t,this._renderCube(this._getSourceTarget(),0,e)}_applyPMREM(t){if(this._ggxMaterial===null)this._ggxMaterial=tp(),this._integrationMaterial=ep();let e=this._cubeSize,n=t.texture.mipmaps.length-1,i=this._ggxMaterial.uniforms;i.envMap.value=this._sourceTarget.texture;let s=this._integrationMaterial.uniforms;s.envMap.value=this._sourceTarget.texture,s.sourceLod.value=Math.log2(e/jc);for(let r=0;r<=n;r++){let a=wr.lodToRoughness(r,n);if(r>n-Kf)s.roughness.value=a,this._renderCube(t,r,this._integrationMaterial);else{let o=a>0?Math.log2(e)+0.5*Math.log2(6/(Qc*Math.pow(a,4)))+0.5:0;i.roughness.value=a,i.lodBias.value=o,this._renderCube(t,r,this._ggxMaterial)}}}_blur(t,e){if(this._blurMaterial===null)this._blurMaterial=np();let n=this._blurMaterial,i=n.uniforms,s=this._sourceTarget;i.sigma.value=Math.min(e,Math.PI)/Math.SQRT2,i.envMap.value=s.texture,this._renderCube(t,0,n),i.envMap.value=t.texture,this._renderCube(s,0,n)}}function Ic(t,e,n){return new Ar(t,{minFilter:Yn,generateMipmaps:e,type:bn,colorSpace:ja,depthBuffer:n})}function xs(t,e,n){return new vn({name:t,uniforms:e,vertexShader:ne.cube_vert,fragmentShader:n,side:Qe,blending:ai,depthTest:!1,depthWrite:!1})}function tp(){return xs("PMREMGGXConvolution",{envMap:{value:null},roughness:{value:0},lodBias:{value:0}},`

		#define GGX_SAMPLES ${Qc}u

		varying vec3 vWorldDirection;

		uniform samplerCube envMap;
		uniform float roughness;
		uniform float lodBias;

		#include <common>

		// Van der Corput radical inverse
		float radicalInverse_VdC( uint bits ) {

			bits = ( bits << 16u ) | ( bits >> 16u );
			bits = ( ( bits & 0x55555555u ) << 1u ) | ( ( bits & 0xAAAAAAAAu ) >> 1u );
			bits = ( ( bits & 0x33333333u ) << 2u ) | ( ( bits & 0xCCCCCCCCu ) >> 2u );
			bits = ( ( bits & 0x0F0F0F0Fu ) << 4u ) | ( ( bits & 0xF0F0F0F0u ) >> 4u );
			bits = ( ( bits & 0x00FF00FFu ) << 8u ) | ( ( bits & 0xFF00FF00u ) >> 8u );
			return float( bits ) * 2.3283064365386963e-10; // / 0x100000000

		}

		// Hammersley sequence
		vec2 hammersley( uint i, uint N ) {

			return vec2( float( i ) / float( N ), radicalInverse_VdC( i ) );

		}

		void main() {

			vec3 N = normalize( vWorldDirection );

			// For very low roughness, just sample the environment directly
			if ( roughness < 0.001 ) {

				gl_FragColor = vec4( textureLod( envMap, N, 0.0 ).rgb, 1.0 );
				return;

			}

			float alpha = roughness * roughness;
			float alpha2 = alpha * alpha;

			// Tangent space basis for VNDF sampling
			vec3 up = abs( N.z ) < 0.999 ? vec3( 0.0, 0.0, 1.0 ) : vec3( 1.0, 0.0, 0.0 );
			vec3 tangent = normalize( cross( up, N ) );
			vec3 bitangent = cross( N, tangent );

			vec3 prefilteredColor = vec3( 0.0 );
			float totalWeight = 0.0;

			for ( uint i = 0u; i < GGX_SAMPLES; i ++ ) {

				vec2 Xi = hammersley( i, GGX_SAMPLES );

				// With V = N, sample the reflected direction directly.
				float invQ = 1.0 / ( 1.0 - Xi.x + alpha2 * Xi.x );
				float NdotL = ( 1.0 - Xi.x - alpha2 * Xi.x ) * invQ;

				if ( NdotL > 0.0 ) {

					float phi = 2.0 * PI * Xi.y;
					float sinTheta = 2.0 * alpha * sqrt( Xi.x * ( 1.0 - Xi.x ) ) * invQ;
					vec3 L = N * NdotL + ( tangent * cos( phi ) + bitangent * sin( phi ) ) * sinTheta;

					// Match the source mip to the sample's solid angle. The terms
					// independent of the sample are precomputed in lodBias.
					float d = alpha2 * invQ;
					float lod = max( log2( d ) + lodBias, 0.0 );

					// Weight by NdotL for the split-sum approximation
					prefilteredColor += textureLod( envMap, L, lod ).rgb * NdotL;
					totalWeight += NdotL;

				}

			}

			gl_FragColor = vec4( prefilteredColor / totalWeight, 1.0 );

		}
	`)}function ep(){return xs("PMREMGGXIntegration",{envMap:{value:null},roughness:{value:0},sourceLod:{value:0},sourceSize:{value:jc}},`

		varying vec3 vWorldDirection;

		uniform samplerCube envMap;
		uniform float roughness;
		uniform float sourceLod;
		uniform int sourceSize;

		void main() {

			vec3 N = normalize( vWorldDirection );

			float alpha = roughness * roughness;
			float alpha2 = alpha * alpha;

			float texelSize = 2.0 / float( sourceSize );

			vec3 prefilteredColor = vec3( 0.0 );
			float totalWeight = 0.0;

			// Pair opposite texels: only the one in N's hemisphere contributes.
			for ( int face = 0; face < 3; face ++ ) {

				for ( int y = 0; y < sourceSize; y ++ ) {

					for ( int x = 0; x < sourceSize; x ++ ) {

						vec2 uv = ( vec2( x, y ) + 0.5 ) * texelSize - 1.0;
						vec3 texelDirection = face == 0 ? vec3( 1.0, uv ) : ( face == 1 ? vec3( uv.x, 1.0, uv.y ) : vec3( uv, 1.0 ) );

						float invDistance = inversesqrt( 1.0 + dot( uv, uv ) );
						float NdotL = dot( N, texelDirection );
						texelDirection *= NdotL < 0.0 ? - 1.0 : 1.0;
						NdotL = abs( NdotL ) * invDistance;

						// With V = N, NdotH squared is ( 1 + NdotL ) / 2. Common factors
						// in the GGX distribution and texel solid angle cancel when normalized.
						float d = 1.0 + alpha2 + ( alpha2 - 1.0 ) * NdotL;
						float weight = NdotL * invDistance * invDistance * invDistance / ( d * d );

						prefilteredColor += textureLod( envMap, texelDirection, sourceLod ).rgb * weight;
						totalWeight += weight;

					}

				}

			}

			gl_FragColor = vec4( prefilteredColor / totalWeight, 1.0 );

		}
	`)}function np(){return xs("PMREMSphericalGaussianBlur",{envMap:{value:null},sigma:{value:0}},`

		#define SAMPLES ${$f}
		#define GOLDEN_ANGLE 2.39996322973

		varying vec3 vWorldDirection;

		uniform samplerCube envMap;
		uniform float sigma;

		#include <common>

		void main() {

			vec3 outputDirection = normalize( vWorldDirection );

			vec3 up = abs( outputDirection.z ) < 0.999 ? vec3( 0.0, 0.0, 1.0 ) : vec3( 1.0, 0.0, 0.0 );
			vec3 tangent = normalize( cross( up, outputDirection ) );
			vec3 bitangent = cross( outputDirection, tangent );

			// Truncate the kernel at three standard deviations or at the antipode.
			float thetaMax = min( 3.0 * sigma, PI );
			float truncation = 1.0 - exp( - 0.5 * thetaMax * thetaMax / ( sigma * sigma ) );

			vec3 accumColor = vec3( 0.0 );
			float accumWeight = 0.0;

			for ( int i = 0; i < SAMPLES; i ++ ) {

				// Stratified inverse-CDF sampling of the Gaussian, placed on a golden-angle spiral.
				float stratum = ( float( i ) + 0.5 ) / float( SAMPLES );
				float theta = sigma * sqrt( - 2.0 * log( 1.0 - stratum * truncation ) );
				float phi = float( i ) * GOLDEN_ANGLE;

				vec3 offset = cos( phi ) * tangent + sin( phi ) * bitangent;
				vec3 sampleDirection = cos( theta ) * outputDirection + sin( theta ) * offset;

				// Correct the planar sample density to solid angle.
				float weight = sin( theta ) / theta;

				accumColor += weight * textureLod( envMap, sampleDirection, 0.0 ).rgb;
				accumWeight += weight;

			}

			gl_FragColor = vec4( accumColor / accumWeight, 1.0 );

		}
	`)}function Pc(){return xs("PMREMEquirectangularToCubemap",{envMap:{value:null}},`

		varying vec3 vWorldDirection;

		uniform sampler2D envMap;

		#include <common>

		void main() {

			// Average four subpixel samples to preserve small, bright features.
			vec3 direction = normalize( vWorldDirection );
			vec3 dx = dFdx( direction ) * 0.25;
			vec3 dy = dFdy( direction ) * 0.25;

			vec3 color = textureLod( envMap, equirectUv( normalize( direction - dx - dy ) ), 0.0 ).rgb;
			color += textureLod( envMap, equirectUv( normalize( direction + dx - dy ) ), 0.0 ).rgb;
			color += textureLod( envMap, equirectUv( normalize( direction - dx + dy ) ), 0.0 ).rgb;
			color += textureLod( envMap, equirectUv( normalize( direction + dx + dy ) ), 0.0 ).rgb;
			gl_FragColor = vec4( color * 0.25, 1.0 );

		}
	`)}function Lc(){return xs("PMREMCubemapToCubemap",{envMap:{value:null},flipEnvMap:{value:-1}},`

		varying vec3 vWorldDirection;

		uniform samplerCube envMap;
		uniform float flipEnvMap;

		void main() {

			gl_FragColor = vec4( textureCube( envMap, vec3( flipEnvMap * vWorldDirection.x, vWorldDirection.yz ) ).rgb, 1.0 );

		}
	`)}function ip(t){let e=new WeakMap,n=new WeakMap,i=null;function s(u,d=!1){if(u===null||u===void 0)return null;if(d)return a(u);return r(u)}function r(u){if(u&&u.isTexture){let d=u.mapping;if(d===js||d===tr)if(e.has(u)){let v=e.get(u).texture;return o(v,u.mapping)}else{let v=u.image;if(v&&v.height>0){let w=new Ar(v.height);return w.fromEquirectangularTexture(t,u),e.set(u,w),u.addEventListener("dispose",c),o(w.texture,u.mapping)}else return null}}return u}function a(u){if(u&&u.isTexture&&u.isPMREMTexture!==!0){let d=u.mapping,v=d===js||d===tr,w=d===rs||d===ki;if(v||w){let f=n.get(u),m=f!==void 0?f.texture.pmremVersion:0;if(u.isRenderTargetTexture&&u.pmremVersion!==m){if(i===null)i=new wr(t);return f=v?i.fromEquirectangular(u,f):i.fromCubemap(u,f),f.texture.pmremVersion=u.pmremVersion,n.set(u,f),f.texture}else if(f!==void 0)return f.texture;else{let M=u.image;if(v&&M&&M.height>0||w&&M&&l(M)){if(i===null)i=new wr(t);return f=v?i.fromEquirectangular(u):i.fromCubemap(u),f.texture.pmremVersion=u.pmremVersion,n.set(u,f),u.addEventListener("dispose",p),f.texture}else return null}}}return u}function o(u,d){if(d===js)u.mapping=rs;else if(d===tr)u.mapping=ki;return u}function l(u){let d=0,v=6;for(let w=0;w<v;w++)if(u[w]!==void 0)d++;return d===v}function c(u){let d=u.target;d.removeEventListener("dispose",c);let v=e.get(d);if(v!==void 0)e.delete(d),v.dispose()}function p(u){let d=u.target;d.removeEventListener("dispose",p);let v=n.get(d);if(v!==void 0)n.delete(d),v.dispose()}function h(){if(e=new WeakMap,n=new WeakMap,i!==null)i.dispose(),i=null}return{get:s,dispose:h}}function sp(t){let e={};function n(i){if(e[i]!==void 0)return e[i];let s=t.getExtension(i);return e[i]=s,s}return{has:function(i){return n(i)!==null},init:function(){n("EXT_color_buffer_float"),n("WEBGL_clip_cull_distance"),n("OES_texture_float_linear"),n("EXT_color_buffer_half_float"),n("WEBGL_multisampled_render_to_texture"),n("WEBGL_render_shared_exponent")},get:function(i){let s=n(i);if(s===null)es("WebGLRenderer: "+i+" extension not supported.");return s}}}function rp(t,e,n,i){let s={},r=new WeakMap,a=new FinalizationRegistry((v)=>delete s[v]);function o(v){l(v.target)}function l(v){if(v.index!==null)e.remove(v.index);for(let f in v.attributes)e.remove(v.attributes[f]);v.removeEventListener("dispose",o),delete s[v.id],a.unregister(v);let w=r.get(v);if(w)e.remove(w),r.delete(v);if(i.releaseStatesOfGeometry(v),v.isInstancedBufferGeometry===!0)delete v._maxInstanceCount;n.memory.geometries--}function c(v,w){if(s[w.id]!==void 0)return w;return w.addEventListener("dispose",o),s[w.id]=new WeakRef(w),a.register(w,w.id,w),n.memory.geometries++,w}function p(v){let w=v.attributes;for(let f in w)e.update(w[f],t.ARRAY_BUFFER)}function h(v){let w=v.index,f=v.attributes.position,m=0;if(f===void 0)return;let M=w!==null?w.array.length:f.array.length/3-1,S=new(f.count>=65535?fr:dr)(Math.ceil(M/3)*6,1),E=S.array;if(w!==null){let g=w.array;m=w.version;for(let y=0,D=0;y<M;y+=3){let q=g[y+0],N=g[y+1],X=g[y+2];E[D++]=q,E[D++]=N,E[D++]=N,E[D++]=X,E[D++]=X,E[D++]=q}}else{m=f.version;for(let g=0,y=0;g<M;g+=3){let D=g+0,q=g+1,N=g+2;E[y++]=D,E[y++]=q,E[y++]=q,E[y++]=N,E[y++]=N,E[y++]=D}}S.version=m;let P=r.get(v);if(P)e.remove(P);r.set(v,S)}function u(v){let w=r.get(v);if(w){let f=v.index;if(f!==null){if(w.version<f.version)h(v)}}else h(v);return r.get(v)}function d(){for(let v in s){let w=s[v].deref();if(w!==void 0)l(w)}}return{get:c,update:p,getWireframeAttribute:u,dispose:d}}function ap(t,e,n){let i;function s(h){i=h}let r,a;function o(h){r=h.type,a=h.bytesPerElement}function l(h,u){t.drawElements(i,u,r,h*a),n.update(u,i,1)}function c(h,u,d){if(d===0)return;t.drawElementsInstanced(i,u,r,h*a,d),n.update(u,i,d)}function p(h,u,d){if(d===0)return;e.get("WEBGL_multi_draw").multiDrawElementsWEBGL(i,u,0,r,h,0,d);let w=0;for(let f=0;f<d;f++)w+=u[f];n.update(w,i,1)}this.setMode=s,this.setIndex=o,this.render=l,this.renderInstances=c,this.renderMultiDraw=p}function op(t){let e={geometries:0,textures:0},n={frame:0,calls:0,triangles:0,points:0,lines:0};function i(r,a,o){switch(n.calls++,a){case t.TRIANGLES:n.triangles+=o*(r/3);break;case t.LINES:n.lines+=o*(r/2);break;case t.LINE_STRIP:n.lines+=o*(r-1);break;case t.LINE_LOOP:n.lines+=o*r;break;case t.POINTS:n.points+=o*r;break;default:Yt("WebGLInfo: Unknown draw mode:",a);break}}function s(){n.calls=0,n.triangles=0,n.points=0,n.lines=0}return{memory:e,render:n,programs:null,autoReset:!0,reset:s,update:i}}function lp(t,e,n){let i=new WeakMap,s=new Ae;function r(a,o,l){let c=a.morphTargetInfluences,p=o.morphAttributes.position||o.morphAttributes.normal||o.morphAttributes.color,h=p!==void 0?p.length:0,u=i.get(o);if(u===void 0||u.count!==h){let D=function(){g.dispose(),i.delete(o),o.removeEventListener("dispose",D)};if(u!==void 0)u.texture.dispose();let d=o.morphAttributes.position!==void 0,v=o.morphAttributes.normal!==void 0,w=o.morphAttributes.color!==void 0,f=o.morphAttributes.position||[],m=o.morphAttributes.normal||[],M=o.morphAttributes.color||[],T=0;if(d===!0)T=1;if(v===!0)T=2;if(w===!0)T=3;let S=o.attributes.position.count*T,E=1;if(S>e.maxTextureSize)E=Math.ceil(S/e.maxTextureSize),S=e.maxTextureSize;let P=new Float32Array(S*E*4*h),g=new cr(P,S,E,h);g.type=Un,g.needsUpdate=!0;let y=T*4;for(let q=0;q<h;q++){let N=f[q],X=m[q],rt=M[q],R=S*E*4*q;for(let j=0;j<N.count;j++){let tt=j*y;if(d===!0)s.fromBufferAttribute(N,j),P[R+tt+0]=s.x,P[R+tt+1]=s.y,P[R+tt+2]=s.z,P[R+tt+3]=0;if(v===!0)s.fromBufferAttribute(X,j),P[R+tt+4]=s.x,P[R+tt+5]=s.y,P[R+tt+6]=s.z,P[R+tt+7]=0;if(w===!0)s.fromBufferAttribute(rt,j),P[R+tt+8]=s.x,P[R+tt+9]=s.y,P[R+tt+10]=s.z,P[R+tt+11]=rt.itemSize===4?s.w:1}}u={count:h,texture:g,size:new fe(S,E)},i.set(o,u),o.addEventListener("dispose",D)}if(a.isInstancedMesh===!0&&a.morphTexture!==null)l.getUniforms().setValue(t,"morphTexture",a.morphTexture,n);else{let d=0;for(let w=0;w<c.length;w++)d+=c[w];let v=o.morphTargetsRelative?1:1-d;l.getUniforms().setValue(t,"morphTargetBaseInfluence",v),l.getUniforms().setValue(t,"morphTargetInfluences",c)}l.getUniforms().setValue(t,"morphTargetsTexture",u.texture,n),l.getUniforms().setValue(t,"morphTargetsTextureSize",u.size)}return{update:r}}function cp(t,e,n,i,s){let r=new WeakMap;function a(c){let p=s.render.frame,h=c.geometry,u=e.get(c,h);if(r.get(u)!==p)e.update(u),r.set(u,p);if(c.isInstancedMesh){if(c.hasEventListener("dispose",l)===!1)c.addEventListener("dispose",l);if(r.get(c)!==p){if(n.update(c.instanceMatrix,t.ARRAY_BUFFER),c.instanceColor!==null)n.update(c.instanceColor,t.ARRAY_BUFFER);r.set(c,p)}}if(c.isSkinnedMesh){let d=c.skeleton;if(r.get(d)!==p)d.update(),r.set(d,p)}return u}function o(){r=new WeakMap}function l(c){let p=c.target;if(p.removeEventListener("dispose",l),i.releaseStatesOfObject(p),n.remove(p.instanceMatrix),p.instanceColor!==null)n.remove(p.instanceColor)}return{update:a,dispose:o}}var hp={[ca]:"LINEAR_TONE_MAPPING",[ha]:"REINHARD_TONE_MAPPING",[ua]:"CINEON_TONE_MAPPING",[da]:"ACES_FILMIC_TONE_MAPPING",[pa]:"AGX_TONE_MAPPING",[ma]:"NEUTRAL_TONE_MAPPING",[fa]:"CUSTOM_TONE_MAPPING"};function up(t,e,n,i,s,r){let a=new cn(e,n,{type:t,depthBuffer:s,stencilBuffer:r,samples:i?4:0,storeMultisampledDepthBuffer:!1,storeMultisampledStencilBuffer:!1,resolveDepthBuffer:!1,resolveStencilBuffer:!1}),o=null,l=null,c=null,p=new Ee;p.setAttribute("position",new sn([-1,3,0,-1,-1,0,3,-1,0],3)),p.setAttribute("uv",new sn([0,2,0,0,2,0],2));let h=new xo({uniforms:{tDiffuse:{value:null}},vertexShader:`
			precision highp float;

			uniform mat4 modelViewMatrix;
			uniform mat4 projectionMatrix;

			attribute vec3 position;
			attribute vec2 uv;

			varying vec2 vUv;

			void main() {
				vUv = uv;
				gl_Position = projectionMatrix * modelViewMatrix * vec4( position, 1.0 );
			}`,fragmentShader:`
			precision highp float;

			uniform sampler2D tDiffuse;

			varying vec2 vUv;

			#include <tonemapping_pars_fragment>
			#include <colorspace_pars_fragment>

			void main() {
				gl_FragColor = texture2D( tDiffuse, vUv );

				#ifdef LINEAR_TONE_MAPPING
					gl_FragColor.rgb = LinearToneMapping( gl_FragColor.rgb );
				#elif defined( REINHARD_TONE_MAPPING )
					gl_FragColor.rgb = ReinhardToneMapping( gl_FragColor.rgb );
				#elif defined( CINEON_TONE_MAPPING )
					gl_FragColor.rgb = CineonToneMapping( gl_FragColor.rgb );
				#elif defined( ACES_FILMIC_TONE_MAPPING )
					gl_FragColor.rgb = ACESFilmicToneMapping( gl_FragColor.rgb );
				#elif defined( AGX_TONE_MAPPING )
					gl_FragColor.rgb = AgXToneMapping( gl_FragColor.rgb );
				#elif defined( NEUTRAL_TONE_MAPPING )
					gl_FragColor.rgb = NeutralToneMapping( gl_FragColor.rgb );
				#elif defined( CUSTOM_TONE_MAPPING )
					gl_FragColor.rgb = CustomToneMapping( gl_FragColor.rgb );
				#endif

				#ifdef SRGB_TRANSFER
					gl_FragColor = sRGBTransferOETF( gl_FragColor );
				#endif
			}`,depthTest:!1,depthWrite:!1}),u=new hn(p,h),d=new Po(-1,1,1,-1,0,1),v=null,w=null,f=!1,m,M=null,T=[],S=!1;this.setSize=function(E,P){if(a.setSize(E,P),o!==null)o.setSize(E,P);if(l!==null)l.setSize(E,P);for(let g=0;g<T.length;g++){let y=T[g];if(y.setSize)y.setSize(E,P)}},this.setEffects=function(E){T=E,S=T.length>0&&T[0].isRenderPass===!0;let{width:P,height:g}=a;if(T.length>0&&o===null)o=new cn(P,g,{type:bn,depthBuffer:!1,stencilBuffer:!1}),l=new cn(P,g,{type:bn,depthBuffer:!1,stencilBuffer:!1});let y=s===!0&&T.some((D)=>D.needsDepthTexture===!0);if(y!==(a.depthTexture!==null)){if(y===!0&&c===null){if(c=new Kn(P,g),r===!0)c.format=Jn,c.type=li}a.dispose(),a.depthTexture=y?c:null,a.resolveDepthBuffer=y,a.storeMultisampledDepthBuffer=y}for(let D=0;D<T.length;D++){let q=T[D];if(q.setSize)q.setSize(P,g);if(y&&q.needsDepthTexture===!0&&q.setDepthTexture)q.setDepthTexture(c)}},this.begin=function(E,P){if(f)return!1;if(E.toneMapping===Sn&&T.length===0)return!1;if(M=P,P!==null){let{width:g,height:y}=P;if(a.width!==g||a.height!==y)this.setSize(g,y)}if(S===!1)E.setRenderTarget(a);return m=E.toneMapping,E.toneMapping=Sn,!0},this.hasRenderPass=function(){return S},this.end=function(E,P){E.toneMapping=m,f=!0;let g=a,y=o;for(let D=0;D<T.length;D++){let q=T[D];if(q.enabled===!1)continue;if(q.render(E,y,g,P),q.needsSwap!==!1)g=y,y=y===o?l:o}if(v!==E.outputColorSpace||w!==E.toneMapping){if(v=E.outputColorSpace,w=E.toneMapping,h.defines={},ae.getTransfer(v)===ye)h.defines.SRGB_TRANSFER="";let D=hp[w];if(D)h.defines[D]="";h.needsUpdate=!0}h.uniforms.tDiffuse.value=g.texture,E.setRenderTarget(M),E.render(u,d),M=null,f=!1},this.isCompositing=function(){return f},this.dispose=function(){if(a.dispose(),o!==null)o.dispose();if(l!==null)l.dispose();p.dispose(),h.dispose()}}var th=new Ve,eh=new Kn(1,1),nh=new xr(1),ih=new cr,sh=new ho,rh=new _r,Nc=[],Dc=[],Uc=new Float32Array(16),Fc=new Float32Array(9),Oc=new Float32Array(4);function Vi(t,e,n){let i=t[0];if(i<=0||i>0)return t;let s=e*n,r=Nc[s];if(r===void 0)r=new Float32Array(s),Nc[s]=r;if(e!==0){i.toArray(r,0);for(let a=1,o=0;a!==e;++a)o+=n,t[a].toArray(r,o)}return r}function Ne(t,e){if(t.length!==e.length)return!1;for(let n=0,i=t.length;n<i;n++)if(t[n]!==e[n])return!1;return!0}function De(t,e){for(let n=0,i=e.length;n<i;n++)t[n]=e[n]}function Tr(t,e){let n=Dc[e];if(n===void 0)n=new Int32Array(e),Dc[e]=n;for(let i=0;i!==e;++i)n[i]=t.allocateTextureUnit();return n}function dp(t,e){let n=this.cache;if(n[0]===e)return;t.uniform1f(this.addr,e),n[0]=e}function fp(t,e){let n=this.cache;if(e.x!==void 0){if(n[0]!==e.x||n[1]!==e.y)t.uniform2f(this.addr,e.x,e.y),n[0]=e.x,n[1]=e.y}else{if(Ne(n,e))return;t.uniform2fv(this.addr,e),De(n,e)}}function pp(t,e){let n=this.cache;if(e.x!==void 0){if(n[0]!==e.x||n[1]!==e.y||n[2]!==e.z)t.uniform3f(this.addr,e.x,e.y,e.z),n[0]=e.x,n[1]=e.y,n[2]=e.z}else if(e.r!==void 0){if(n[0]!==e.r||n[1]!==e.g||n[2]!==e.b)t.uniform3f(this.addr,e.r,e.g,e.b),n[0]=e.r,n[1]=e.g,n[2]=e.b}else{if(Ne(n,e))return;t.uniform3fv(this.addr,e),De(n,e)}}function mp(t,e){let n=this.cache;if(e.x!==void 0){if(n[0]!==e.x||n[1]!==e.y||n[2]!==e.z||n[3]!==e.w)t.uniform4f(this.addr,e.x,e.y,e.z,e.w),n[0]=e.x,n[1]=e.y,n[2]=e.z,n[3]=e.w}else{if(Ne(n,e))return;t.uniform4fv(this.addr,e),De(n,e)}}function gp(t,e){let n=this.cache,i=e.elements;if(i===void 0){if(Ne(n,e))return;t.uniformMatrix2fv(this.addr,!1,e),De(n,e)}else{if(Ne(n,i))return;Oc.set(i),t.uniformMatrix2fv(this.addr,!1,Oc),De(n,i)}}function _p(t,e){let n=this.cache,i=e.elements;if(i===void 0){if(Ne(n,e))return;t.uniformMatrix3fv(this.addr,!1,e),De(n,e)}else{if(Ne(n,i))return;Fc.set(i),t.uniformMatrix3fv(this.addr,!1,Fc),De(n,i)}}function xp(t,e){let n=this.cache,i=e.elements;if(i===void 0){if(Ne(n,e))return;t.uniformMatrix4fv(this.addr,!1,e),De(n,e)}else{if(Ne(n,i))return;Uc.set(i),t.uniformMatrix4fv(this.addr,!1,Uc),De(n,i)}}function vp(t,e){let n=this.cache;if(n[0]===e)return;t.uniform1i(this.addr,e),n[0]=e}function yp(t,e){let n=this.cache;if(e.x!==void 0){if(n[0]!==e.x||n[1]!==e.y)t.uniform2i(this.addr,e.x,e.y),n[0]=e.x,n[1]=e.y}else{if(Ne(n,e))return;t.uniform2iv(this.addr,e),De(n,e)}}function Mp(t,e){let n=this.cache;if(e.x!==void 0){if(n[0]!==e.x||n[1]!==e.y||n[2]!==e.z)t.uniform3i(this.addr,e.x,e.y,e.z),n[0]=e.x,n[1]=e.y,n[2]=e.z}else{if(Ne(n,e))return;t.uniform3iv(this.addr,e),De(n,e)}}function Sp(t,e){let n=this.cache;if(e.x!==void 0){if(n[0]!==e.x||n[1]!==e.y||n[2]!==e.z||n[3]!==e.w)t.uniform4i(this.addr,e.x,e.y,e.z,e.w),n[0]=e.x,n[1]=e.y,n[2]=e.z,n[3]=e.w}else{if(Ne(n,e))return;t.uniform4iv(this.addr,e),De(n,e)}}function bp(t,e){let n=this.cache;if(n[0]===e)return;t.uniform1ui(this.addr,e),n[0]=e}function wp(t,e){let n=this.cache;if(e.x!==void 0){if(n[0]!==e.x||n[1]!==e.y)t.uniform2ui(this.addr,e.x,e.y),n[0]=e.x,n[1]=e.y}else{if(Ne(n,e))return;t.uniform2uiv(this.addr,e),De(n,e)}}function Ap(t,e){let n=this.cache;if(e.x!==void 0){if(n[0]!==e.x||n[1]!==e.y||n[2]!==e.z)t.uniform3ui(this.addr,e.x,e.y,e.z),n[0]=e.x,n[1]=e.y,n[2]=e.z}else{if(Ne(n,e))return;t.uniform3uiv(this.addr,e),De(n,e)}}function Tp(t,e){let n=this.cache;if(e.x!==void 0){if(n[0]!==e.x||n[1]!==e.y||n[2]!==e.z||n[3]!==e.w)t.uniform4ui(this.addr,e.x,e.y,e.z,e.w),n[0]=e.x,n[1]=e.y,n[2]=e.z,n[3]=e.w}else{if(Ne(n,e))return;t.uniform4uiv(this.addr,e),De(n,e)}}function Er(t){if(t.version===0)t.compareFunction=io,t.needsUpdate=!0;return t}function Ep(t,e,n){let i=this.cache,s=n.allocateTextureUnit();if(i[0]!==s)t.uniform1i(this.addr,s),i[0]=s;let r;if(this.type===t.SAMPLER_2D_SHADOW)r=Er(eh);else r=th;n.setTexture2D(e||r,s)}function Rp(t,e,n){let i=this.cache,s=n.allocateTextureUnit();if(i[0]!==s)t.uniform1i(this.addr,s),i[0]=s;n.setTexture3D(e||sh,s)}function Cp(t,e,n){let i=this.cache,s=n.allocateTextureUnit();if(i[0]!==s)t.uniform1i(this.addr,s),i[0]=s;let r;if(this.type===t.SAMPLER_CUBE_SHADOW)r=Er(nh);else r=rh;n.setTextureCube(e||r,s)}function Ip(t,e,n){let i=this.cache,s=n.allocateTextureUnit();if(i[0]!==s)t.uniform1i(this.addr,s),i[0]=s;n.setTexture2DArray(e||ih,s)}function Pp(t){switch(t){case 5126:return dp;case 35664:return fp;case 35665:return pp;case 35666:return mp;case 35674:return gp;case 35675:return _p;case 35676:return xp;case 5124:case 35670:return vp;case 35667:case 35671:return yp;case 35668:case 35672:return Mp;case 35669:case 35673:return Sp;case 5125:return bp;case 36294:return wp;case 36295:return Ap;case 36296:return Tp;case 35678:case 36198:case 36298:case 36306:case 35682:return Ep;case 35679:case 36299:case 36307:return Rp;case 35680:case 36300:case 36308:case 36293:return Cp;case 36289:case 36303:case 36311:case 36292:return Ip}}function Lp(t,e){t.uniform1fv(this.addr,e)}function Np(t,e){let n=Vi(e,this.size,2);t.uniform2fv(this.addr,n)}function Dp(t,e){let n=Vi(e,this.size,3);t.uniform3fv(this.addr,n)}function Up(t,e){let n=Vi(e,this.size,4);t.uniform4fv(this.addr,n)}function Fp(t,e){let n=Vi(e,this.size,4);t.uniformMatrix2fv(this.addr,!1,n)}function Op(t,e){let n=Vi(e,this.size,9);t.uniformMatrix3fv(this.addr,!1,n)}function Bp(t,e){let n=Vi(e,this.size,16);t.uniformMatrix4fv(this.addr,!1,n)}function zp(t,e){t.uniform1iv(this.addr,e)}function kp(t,e){t.uniform2iv(this.addr,e)}function Gp(t,e){t.uniform3iv(this.addr,e)}function Hp(t,e){t.uniform4iv(this.addr,e)}function Wp(t,e){t.uniform1uiv(this.addr,e)}function Vp(t,e){t.uniform2uiv(this.addr,e)}function Xp(t,e){t.uniform3uiv(this.addr,e)}function qp(t,e){t.uniform4uiv(this.addr,e)}function Yp(t,e,n){let i=this.cache,s=e.length,r=Tr(n,s);if(!Ne(i,r))t.uniform1iv(this.addr,r),De(i,r);let a;if(this.type===t.SAMPLER_2D_SHADOW)a=Er(eh);else a=th;for(let o=0;o!==s;++o)n.setTexture2D(e[o]||a,r[o])}function Zp(t,e,n){let i=this.cache,s=e.length,r=Tr(n,s);if(!Ne(i,r))t.uniform1iv(this.addr,r),De(i,r);for(let a=0;a!==s;++a)n.setTexture3D(e[a]||sh,r[a])}function Jp(t,e,n){let i=this.cache,s=e.length,r=Tr(n,s);if(!Ne(i,r))t.uniform1iv(this.addr,r),De(i,r);let a;if(this.type===t.SAMPLER_CUBE_SHADOW)a=Er(nh);else a=rh;for(let o=0;o!==s;++o)n.setTextureCube(e[o]||a,r[o])}function $p(t,e,n){let i=this.cache,s=e.length,r=Tr(n,s);if(!Ne(i,r))t.uniform1iv(this.addr,r),De(i,r);for(let a=0;a!==s;++a)n.setTexture2DArray(e[a]||ih,r[a])}function Kp(t){switch(t){case 5126:return Lp;case 35664:return Np;case 35665:return Dp;case 35666:return Up;case 35674:return Fp;case 35675:return Op;case 35676:return Bp;case 5124:case 35670:return zp;case 35667:case 35671:return kp;case 35668:case 35672:return Gp;case 35669:case 35673:return Hp;case 5125:return Wp;case 36294:return Vp;case 36295:return Xp;case 36296:return qp;case 35678:case 36198:case 36298:case 36306:case 35682:return Yp;case 35679:case 36299:case 36307:return Zp;case 35680:case 36300:case 36308:case 36293:return Jp;case 36289:case 36303:case 36311:case 36292:return $p}}class ah{constructor(t,e,n){this.id=t,this.addr=n,this.cache=[],this.type=e.type,this.setValue=Pp(e.type)}}class oh{constructor(t,e,n){this.id=t,this.addr=n,this.cache=[],this.type=e.type,this.size=e.size,this.setValue=Kp(e.type)}}class lh{constructor(t){this.id=t,this.seq=[],this.map={}}setValue(t,e,n){let i=this.seq;for(let s=0,r=i.length;s!==r;++s){let a=i[s];a.setValue(t,e[a.id],n)}}}var Fo=/(\w+)(\])?(\[|\.)?/g;function Bc(t,e){t.seq.push(e),t.map[e.id]=e}function Qp(t,e,n){let i=t.name,s=i.length;Fo.lastIndex=0;while(!0){let r=Fo.exec(i),a=Fo.lastIndex,o=r[1],l=r[2]==="]",c=r[3];if(l)o=o|0;if(c===void 0||c==="["&&a+2===s){Bc(n,c===void 0?new ah(o,t,e):new oh(o,t,e));break}else{let h=n.map[o];if(h===void 0)h=new lh(o),Bc(n,h);n=h}}}class _s{constructor(t,e){this.seq=[],this.map={};let n=t.getProgramParameter(e,t.ACTIVE_UNIFORMS);for(let r=0;r<n;++r){let a=t.getActiveUniform(e,r),o=t.getUniformLocation(e,a.name);Qp(a,o,this)}let i=[],s=[];for(let r of this.seq)if(r.type===t.SAMPLER_2D_SHADOW||r.type===t.SAMPLER_CUBE_SHADOW||r.type===t.SAMPLER_2D_ARRAY_SHADOW)i.push(r);else s.push(r);if(i.length>0)this.seq=i.concat(s)}setValue(t,e,n,i){let s=this.map[e];if(s!==void 0)s.setValue(t,n,i)}setOptional(t,e,n){let i=e[n];if(i!==void 0)this.setValue(t,n,i)}static upload(t,e,n,i){for(let s=0,r=e.length;s!==r;++s){let a=e[s],o=n[a.id];if(o.needsUpdate!==!1)a.setValue(t,o.value,i)}}static seqWithValue(t,e){let n=[];for(let i=0,s=t.length;i!==s;++i){let r=t[i];if(r.id in e)n.push(r)}return n}}function zc(t,e,n){let i=t.createShader(e);return t.shaderSource(i,n),t.compileShader(i),i}var jp=37297,tm=0;function em(t,e){let n=t.split(`
`),i=[],s=Math.max(e-6,0),r=Math.min(e+6,n.length);for(let a=s;a<r;a++){let o=a+1;i.push(`${o===e?">":" "} ${o}: ${n[a]}`)}return i.join(`
`)}var kc=new Zt;function nm(t){ae._getMatrix(kc,ae.workingColorSpace,t);let e=`mat3( ${kc.elements.map((n)=>n.toFixed(4))} )`;switch(ae.getTransfer(t)){case to:return[e,"LinearTransferOETF"];case ye:return[e,"sRGBTransferOETF"];default:return Wt("WebGLProgram: Unsupported color space: ",t),[e,"LinearTransferOETF"]}}function Gc(t,e,n){let i=t.getShaderParameter(e,t.COMPILE_STATUS),r=(t.getShaderInfoLog(e)||"").trim();if(i&&r==="")return"";let a=/ERROR: 0:(\d+)/.exec(r);if(a){let o=parseInt(a[1]);return n.toUpperCase()+`

`+r+`

`+em(t.getShaderSource(e),o)}else return r}function im(t,e){let n=nm(e);return[`vec4 ${t}( vec4 value ) {`,`	return ${n[1]}( vec4( ${n[0]} * value.rgb, value.a ) );`,"}"].join(`
`)}var sm={[ca]:"Linear",[ha]:"Reinhard",[ua]:"Cineon",[da]:"ACESFilmic",[pa]:"AgX",[ma]:"Neutral",[fa]:"Custom"};function rm(t,e){let n=sm[e];if(n===void 0)return Wt("WebGLProgram: Unsupported toneMapping:",e),"vec3 "+t+"( vec3 color ) { return LinearToneMapping( color ); }";return"vec3 "+t+"( vec3 color ) { return "+n+"ToneMapping( color ); }"}var br=new W;function am(){ae.getLuminanceCoefficients(br);let t=br.x.toFixed(4),e=br.y.toFixed(4),n=br.z.toFixed(4);return["float luminance( const in vec3 rgb ) {",`	const vec3 weights = vec3( ${t}, ${e}, ${n} );`,"\treturn dot( weights, rgb );","}"].join(`
`)}function om(t){return[t.extensionClipCullDistance?"#extension GL_ANGLE_clip_cull_distance : require":"",t.extensionMultiDraw?"#extension GL_ANGLE_multi_draw : require":""].filter(gs).join(`
`)}function lm(t){let e=[];for(let n in t){let i=t[n];if(i===!1)continue;e.push("#define "+n+" "+i)}return e.join(`
`)}function cm(t,e){let n={},i=t.getProgramParameter(e,t.ACTIVE_ATTRIBUTES);for(let s=0;s<i;s++){let r=t.getActiveAttrib(e,s),a=r.name,o=1;if(r.type===t.FLOAT_MAT2)o=2;if(r.type===t.FLOAT_MAT3)o=3;if(r.type===t.FLOAT_MAT4)o=4;n[a]={type:r.type,location:t.getAttribLocation(e,a),locationSize:o}}return n}function gs(t){return t!==""}function Hc(t,e){let n=e.numSpotLightShadows+e.numSpotLightMaps-e.numSpotLightShadowsWithMaps;return t.replace(/NUM_SUN_LIGHTS/g,e.numSunLights).replace(/NUM_DIR_LIGHTS/g,e.numDirLights).replace(/NUM_SPOT_LIGHTS/g,e.numSpotLights).replace(/NUM_SPOT_LIGHT_MAPS/g,e.numSpotLightMaps).replace(/NUM_SPOT_LIGHT_COORDS/g,n).replace(/NUM_RECT_AREA_LIGHTS/g,e.numRectAreaLights).replace(/NUM_POINT_LIGHTS/g,e.numPointLights).replace(/NUM_HEMI_LIGHTS/g,e.numHemiLights).replace(/NUM_SUN_LIGHT_SHADOWS/g,e.numSunLightShadows).replace(/NUM_DIR_LIGHT_SHADOWS/g,e.numDirLightShadows).replace(/NUM_SPOT_LIGHT_SHADOWS_WITH_MAPS/g,e.numSpotLightShadowsWithMaps).replace(/NUM_SPOT_LIGHT_SHADOWS/g,e.numSpotLightShadows).replace(/NUM_POINT_LIGHT_SHADOWS/g,e.numPointLightShadows).replace(/NUM_LIGHT_PROBE_GRIDS/g,e.numLightProbeGrids)}function Wc(t,e){return t.replace(/NUM_CLIPPING_PLANES/g,e.numClippingPlanes).replace(/UNION_CLIPPING_PLANES/g,e.numClippingPlanes-e.numClipIntersection)}var hm=/^[ \t]*#include +<([\w\d./]+)>/gm;function Bo(t){return t.replace(hm,um)}function um(t,e){let n=ne[e];if(n===void 0)throw Error("THREE.WebGLProgram: Can not resolve #include <"+e+">");return Bo(n)}var dm=/#pragma unroll_loop_start\s+for\s*\(\s*int\s+i\s*=\s*(\d+)\s*;\s*i\s*<\s*(\d+)\s*;\s*i\s*\+\+\s*\)\s*{([\s\S]+?)}\s+#pragma unroll_loop_end/g;function Vc(t){return t.replace(dm,fm)}function fm(t,e,n,i){let s="";for(let r=parseInt(e);r<parseInt(n);r++)s+=i.replace(/\[\s*i\s*\]/g,"[ "+r+" ]").replace(/UNROLLED_LOOP_INDEX/g,r);return s}function Xc(t){let e=`precision ${t.precision} float;
	precision ${t.precision} int;
	precision ${t.precision} sampler2D;
	precision ${t.precision} samplerCube;
	precision ${t.precision} sampler3D;
	precision ${t.precision} sampler2DArray;
	precision ${t.precision} sampler2DShadow;
	precision ${t.precision} samplerCubeShadow;
	precision ${t.precision} sampler2DArrayShadow;
	precision ${t.precision} isampler2D;
	precision ${t.precision} isampler3D;
	precision ${t.precision} isamplerCube;
	precision ${t.precision} isampler2DArray;
	precision ${t.precision} usampler2D;
	precision ${t.precision} usampler3D;
	precision ${t.precision} usamplerCube;
	precision ${t.precision} usampler2DArray;
	`;if(t.precision==="highp")e+=`
#define HIGH_PRECISION`;else if(t.precision==="mediump")e+=`
#define MEDIUM_PRECISION`;else if(t.precision==="lowp")e+=`
#define LOW_PRECISION`;return e}var pm={[is]:"SHADOWMAP_TYPE_PCF",[Oi]:"SHADOWMAP_TYPE_VSM"};function mm(t){return pm[t.shadowMapType]||"SHADOWMAP_TYPE_BASIC"}var gm={[ki]:"ENVMAP_MODE_REFRACTION"};function _m(t){if(t.envMap===!1)return"ENVMAP_MODE_REFLECTION";return gm[t.envMapMode]||"ENVMAP_MODE_REFLECTION"}var xm={[Ql]:"ENVMAP_BLENDING_MULTIPLY",[jl]:"ENVMAP_BLENDING_MIX",[tc]:"ENVMAP_BLENDING_ADD"};function vm(t){if(t.envMap===!1)return"ENVMAP_BLENDING_NONE";return xm[t.combine]||"ENVMAP_BLENDING_NONE"}function ym(t,e,n,i){let s=t.getContext(),{defines:r,vertexShader:a,fragmentShader:o}=n,l=mm(n),c=n.envMapPMREM===!0?"ENVMAP_TYPE_PMREM":"ENVMAP_TYPE_CUBE",p=_m(n),h=vm(n),u=om(n),d=lm(r),v=s.createProgram(),w,f,m=n.glslVersion?"#version "+n.glslVersion+`
`:"";if(n.isRawShaderMaterial){if(w=["#define SHADER_TYPE "+n.shaderType,"#define SHADER_NAME "+n.shaderName,d].filter(gs).join(`
`),w.length>0)w+=`
`;if(f=["#define SHADER_TYPE "+n.shaderType,"#define SHADER_NAME "+n.shaderName,d].filter(gs).join(`
`),f.length>0)f+=`
`}else w=[Xc(n),"#define SHADER_TYPE "+n.shaderType,"#define SHADER_NAME "+n.shaderName,d,n.extensionClipCullDistance?"#define USE_CLIP_DISTANCE":"",n.batching?"#define USE_BATCHING":"",n.batchingColor?"#define USE_BATCHING_COLOR":"",n.instancing?"#define USE_INSTANCING":"",n.instancingColor?"#define USE_INSTANCING_COLOR":"",n.instancingMorph?"#define USE_INSTANCING_MORPH":"",n.useFog&&n.fog?"#define USE_FOG":"",n.useFog&&n.fogExp2?"#define FOG_EXP2":"",n.map?"#define USE_MAP":"",n.envMap?"#define USE_ENVMAP":"",n.envMap?"#define "+p:"",n.lightMap?"#define USE_LIGHTMAP":"",n.aoMap?"#define USE_AOMAP":"",n.bumpMap?"#define USE_BUMPMAP":"",n.normalMap?"#define USE_NORMALMAP":"",n.normalMapObjectSpace?"#define USE_NORMALMAP_OBJECTSPACE":"",n.normalMapTangentSpace?"#define USE_NORMALMAP_TANGENTSPACE":"",n.displacementMap?"#define USE_DISPLACEMENTMAP":"",n.emissiveMap?"#define USE_EMISSIVEMAP":"",n.anisotropy?"#define USE_ANISOTROPY":"",n.anisotropyMap?"#define USE_ANISOTROPYMAP":"",n.clearcoatMap?"#define USE_CLEARCOATMAP":"",n.clearcoatRoughnessMap?"#define USE_CLEARCOAT_ROUGHNESSMAP":"",n.clearcoatNormalMap?"#define USE_CLEARCOAT_NORMALMAP":"",n.diffuseRoughnessMap?"#define USE_DIFFUSE_ROUGHNESSMAP":"",n.iridescenceMap?"#define USE_IRIDESCENCEMAP":"",n.iridescenceThicknessMap?"#define USE_IRIDESCENCE_THICKNESSMAP":"",n.specularMap?"#define USE_SPECULARMAP":"",n.specularColorMap?"#define USE_SPECULAR_COLORMAP":"",n.specularIntensityMap?"#define USE_SPECULAR_INTENSITYMAP":"",n.roughnessMap?"#define USE_ROUGHNESSMAP":"",n.metalnessMap?"#define USE_METALNESSMAP":"",n.alphaMap?"#define USE_ALPHAMAP":"",n.alphaHash?"#define USE_ALPHAHASH":"",n.transmission?"#define USE_TRANSMISSION":"",n.transmissionMap?"#define USE_TRANSMISSIONMAP":"",n.thicknessMap?"#define USE_THICKNESSMAP":"",n.sheenColorMap?"#define USE_SHEEN_COLORMAP":"",n.sheenRoughnessMap?"#define USE_SHEEN_ROUGHNESSMAP":"",n.mapUv?"#define MAP_UV "+n.mapUv:"",n.alphaMapUv?"#define ALPHAMAP_UV "+n.alphaMapUv:"",n.lightMapUv?"#define LIGHTMAP_UV "+n.lightMapUv:"",n.aoMapUv?"#define AOMAP_UV "+n.aoMapUv:"",n.emissiveMapUv?"#define EMISSIVEMAP_UV "+n.emissiveMapUv:"",n.bumpMapUv?"#define BUMPMAP_UV "+n.bumpMapUv:"",n.normalMapUv?"#define NORMALMAP_UV "+n.normalMapUv:"",n.displacementMapUv?"#define DISPLACEMENTMAP_UV "+n.displacementMapUv:"",n.metalnessMapUv?"#define METALNESSMAP_UV "+n.metalnessMapUv:"",n.roughnessMapUv?"#define ROUGHNESSMAP_UV "+n.roughnessMapUv:"",n.anisotropyMapUv?"#define ANISOTROPYMAP_UV "+n.anisotropyMapUv:"",n.clearcoatMapUv?"#define CLEARCOATMAP_UV "+n.clearcoatMapUv:"",n.clearcoatNormalMapUv?"#define CLEARCOAT_NORMALMAP_UV "+n.clearcoatNormalMapUv:"",n.clearcoatRoughnessMapUv?"#define CLEARCOAT_ROUGHNESSMAP_UV "+n.clearcoatRoughnessMapUv:"",n.diffuseRoughnessMapUv?"#define DIFFUSE_ROUGHNESSMAP_UV "+n.diffuseRoughnessMapUv:"",n.iridescenceMapUv?"#define IRIDESCENCEMAP_UV "+n.iridescenceMapUv:"",n.iridescenceThicknessMapUv?"#define IRIDESCENCE_THICKNESSMAP_UV "+n.iridescenceThicknessMapUv:"",n.sheenColorMapUv?"#define SHEEN_COLORMAP_UV "+n.sheenColorMapUv:"",n.sheenRoughnessMapUv?"#define SHEEN_ROUGHNESSMAP_UV "+n.sheenRoughnessMapUv:"",n.specularMapUv?"#define SPECULARMAP_UV "+n.specularMapUv:"",n.specularColorMapUv?"#define SPECULAR_COLORMAP_UV "+n.specularColorMapUv:"",n.specularIntensityMapUv?"#define SPECULAR_INTENSITYMAP_UV "+n.specularIntensityMapUv:"",n.transmissionMapUv?"#define TRANSMISSIONMAP_UV "+n.transmissionMapUv:"",n.thicknessMapUv?"#define THICKNESSMAP_UV "+n.thicknessMapUv:"",n.vertexTangents&&n.flatShading===!1?"#define USE_TANGENT":"",n.vertexNormals?"#define HAS_NORMAL":"",n.vertexColors?"#define USE_COLOR":"",n.vertexAlphas?"#define USE_COLOR_ALPHA":"",n.vertexUv1s?"#define USE_UV1":"",n.vertexUv2s?"#define USE_UV2":"",n.vertexUv3s?"#define USE_UV3":"",n.pointsUvs?"#define USE_POINTS_UV":"",n.flatShading?"#define FLAT_SHADED":"",n.skinning?"#define USE_SKINNING":"",n.morphTargets?"#define USE_MORPHTARGETS":"",n.morphNormals&&n.flatShading===!1?"#define USE_MORPHNORMALS":"",n.morphColors?"#define USE_MORPHCOLORS":"",n.morphTargetsCount>0?"#define MORPHTARGETS_TEXTURE_STRIDE "+n.morphTextureStride:"",n.morphTargetsCount>0?"#define MORPHTARGETS_COUNT "+n.morphTargetsCount:"",n.doubleSided?"#define DOUBLE_SIDED":"",n.flipSided?"#define FLIP_SIDED":"",n.shadowMapEnabled?"#define USE_SHADOWMAP":"",n.shadowMapEnabled?"#define "+l:"",n.sizeAttenuation?"#define USE_SIZEATTENUATION":"",n.numLightProbes>0?"#define USE_LIGHT_PROBES":"",n.logarithmicDepthBuffer?"#define USE_LOGARITHMIC_DEPTH_BUFFER":"",n.reversedDepthBuffer?"#define USE_REVERSED_DEPTH_BUFFER":"","uniform mat4 modelMatrix;","uniform mat4 modelViewMatrix;","uniform mat4 projectionMatrix;","uniform mat4 viewMatrix;","uniform mat3 normalMatrix;","uniform vec3 cameraPosition;","uniform bool isOrthographic;","#ifdef USE_INSTANCING","\tattribute mat4 instanceMatrix;","#endif","#ifdef USE_INSTANCING_COLOR","\tattribute vec3 instanceColor;","#endif","#ifdef USE_INSTANCING_MORPH","\tuniform sampler2D morphTexture;","#endif","attribute vec3 position;","attribute vec3 normal;","attribute vec2 uv;","#ifdef USE_UV1","\tattribute vec2 uv1;","#endif","#ifdef USE_UV2","\tattribute vec2 uv2;","#endif","#ifdef USE_UV3","\tattribute vec2 uv3;","#endif","#ifdef USE_TANGENT","\tattribute vec4 tangent;","#endif","#if defined( USE_COLOR_ALPHA )","\tattribute vec4 color;","#elif defined( USE_COLOR )","\tattribute vec3 color;","#endif","#ifdef USE_SKINNING","\tattribute vec4 skinIndex;","\tattribute vec4 skinWeight;","#endif",`
`].filter(gs).join(`
`),f=[Xc(n),"#define SHADER_TYPE "+n.shaderType,"#define SHADER_NAME "+n.shaderName,d,n.useFog&&n.fog?"#define USE_FOG":"",n.useFog&&n.fogExp2?"#define FOG_EXP2":"",n.alphaToCoverage?"#define ALPHA_TO_COVERAGE":"",n.map?"#define USE_MAP":"",n.matcap?"#define USE_MATCAP":"",n.envMap?"#define USE_ENVMAP":"",n.envMap?"#define "+c:"",n.envMap?"#define "+p:"",n.envMap?"#define "+h:"",n.envMapPMREM?"#define ENVMAP_MAX_LOD "+n.envMapMaxLod+".0":"",n.envMapPMREM?"#define ENVMAP_SIZE "+n.envMapSize+".0":"",n.lightMap?"#define USE_LIGHTMAP":"",n.aoMap?"#define USE_AOMAP":"",n.bumpMap?"#define USE_BUMPMAP":"",n.normalMap?"#define USE_NORMALMAP":"",n.normalMapObjectSpace?"#define USE_NORMALMAP_OBJECTSPACE":"",n.normalMapTangentSpace?"#define USE_NORMALMAP_TANGENTSPACE":"",n.packedNormalMap?"#define USE_PACKED_NORMALMAP":"",n.emissiveMap?"#define USE_EMISSIVEMAP":"",n.anisotropy?"#define USE_ANISOTROPY":"",n.anisotropyMap?"#define USE_ANISOTROPYMAP":"",n.clearcoat?"#define USE_CLEARCOAT":"",n.clearcoatMap?"#define USE_CLEARCOATMAP":"",n.clearcoatRoughnessMap?"#define USE_CLEARCOAT_ROUGHNESSMAP":"",n.clearcoatNormalMap?"#define USE_CLEARCOAT_NORMALMAP":"",n.diffuseRoughness?"#define USE_DIFFUSE_ROUGHNESS":"",n.diffuseRoughnessMap?"#define USE_DIFFUSE_ROUGHNESSMAP":"",n.dispersion?"#define USE_DISPERSION":"",n.retroreflection?"#define USE_RETROREFLECTION":"",n.iridescence?"#define USE_IRIDESCENCE":"",n.iridescenceMap?"#define USE_IRIDESCENCEMAP":"",n.iridescenceThicknessMap?"#define USE_IRIDESCENCE_THICKNESSMAP":"",n.specularMap?"#define USE_SPECULARMAP":"",n.specularColorMap?"#define USE_SPECULAR_COLORMAP":"",n.specularIntensityMap?"#define USE_SPECULAR_INTENSITYMAP":"",n.roughnessMap?"#define USE_ROUGHNESSMAP":"",n.metalnessMap?"#define USE_METALNESSMAP":"",n.alphaMap?"#define USE_ALPHAMAP":"",n.alphaTest?"#define USE_ALPHATEST":"",n.alphaHash?"#define USE_ALPHAHASH":"",n.sheen?"#define USE_SHEEN":"",n.sheenColorMap?"#define USE_SHEEN_COLORMAP":"",n.sheenRoughnessMap?"#define USE_SHEEN_ROUGHNESSMAP":"",n.transmission?"#define USE_TRANSMISSION":"",n.transmissionMap?"#define USE_TRANSMISSIONMAP":"",n.thicknessMap?"#define USE_THICKNESSMAP":"",n.vertexTangents&&n.flatShading===!1?"#define USE_TANGENT":"",n.vertexColors||n.instancingColor?"#define USE_COLOR":"",n.vertexAlphas||n.batchingColor?"#define USE_COLOR_ALPHA":"",n.vertexUv1s?"#define USE_UV1":"",n.vertexUv2s?"#define USE_UV2":"",n.vertexUv3s?"#define USE_UV3":"",n.pointsUvs?"#define USE_POINTS_UV":"",n.gradientMap?"#define USE_GRADIENTMAP":"",n.flatShading?"#define FLAT_SHADED":"",n.doubleSided?"#define DOUBLE_SIDED":"",n.flipSided?"#define FLIP_SIDED":"",n.shadowMapEnabled?"#define USE_SHADOWMAP":"",n.shadowMapEnabled?"#define "+l:"",n.premultipliedAlpha?"#define PREMULTIPLIED_ALPHA":"",n.numLightProbes>0?"#define USE_LIGHT_PROBES":"",n.decodeVideoTexture?"#define DECODE_VIDEO_TEXTURE":"",n.decodeVideoTextureEmissive?"#define DECODE_VIDEO_TEXTURE_EMISSIVE":"",n.logarithmicDepthBuffer?"#define USE_LOGARITHMIC_DEPTH_BUFFER":"",n.reversedDepthBuffer?"#define USE_REVERSED_DEPTH_BUFFER":"","uniform mat4 viewMatrix;","uniform vec3 cameraPosition;","uniform bool isOrthographic;",n.toneMapping!==Sn?"#define TONE_MAPPING":"",n.toneMapping!==Sn?ne.tonemapping_pars_fragment:"",n.toneMapping!==Sn?rm("toneMapping",n.toneMapping):"",n.dithering?"#define DITHERING":"",n.opaque?"#define OPAQUE":"",ne.colorspace_pars_fragment,im("linearToOutputTexel",n.outputColorSpace),am(),n.useDepthPacking?"#define DEPTH_PACKING "+n.depthPacking:"",`
`].filter(gs).join(`
`);if(a=Bo(a),a=Hc(a,n),a=Wc(a,n),o=Bo(o),o=Hc(o,n),o=Wc(o,n),a=Vc(a),o=Vc(o),n.isRawShaderMaterial!==!0)m=`#version 300 es
`,w=[u,"#define attribute in","#define varying out","#define texture2D texture"].join(`
`)+`
`+w,f=["#define varying in",n.glslVersion===so?"":"layout(location = 0) out highp vec4 pc_fragColor;",n.glslVersion===so?"":"#define gl_FragColor pc_fragColor","#define gl_FragDepthEXT gl_FragDepth","#define texture2D texture","#define textureCube texture","#define texture2DProj textureProj","#define texture2DLodEXT textureLod","#define texture2DProjLodEXT textureProjLod","#define textureCubeLodEXT textureLod","#define texture2DGradEXT textureGrad","#define texture2DProjGradEXT textureProjGrad","#define textureCubeGradEXT textureGrad"].join(`
`)+`
`+f;let M=m+w+a,T=m+f+o,S=zc(s,s.VERTEX_SHADER,M),E=zc(s,s.FRAGMENT_SHADER,T);if(s.attachShader(v,S),s.attachShader(v,E),n.index0AttributeName!==void 0)s.bindAttribLocation(v,0,n.index0AttributeName);else if(n.hasPositionAttribute===!0)s.bindAttribLocation(v,0,"position");s.linkProgram(v);function P(q){if(t.debug.checkShaderErrors){let N=s.getProgramInfoLog(v)||"",X=s.getShaderInfoLog(S)||"",rt=s.getShaderInfoLog(E)||"",R=N.trim(),j=X.trim(),tt=rt.trim(),G=!0,pt=!0;if(s.getProgramParameter(v,s.LINK_STATUS)===!1)if(G=!1,typeof t.debug.onShaderError==="function")t.debug.onShaderError(s,v,S,E);else{let J=Gc(s,S,"vertex"),st=Gc(s,E,"fragment");Yt("WebGLProgram: Shader Error "+s.getError()+" - VALIDATE_STATUS "+s.getProgramParameter(v,s.VALIDATE_STATUS)+`

Material Name: `+q.name+`
Material Type: `+q.type+`

Program Info Log: `+R+`
`+J+`
`+st)}else if(R!=="")Wt("WebGLProgram: Program Info Log:",R);else if(j===""||tt==="")pt=!1;if(pt)q.diagnostics={runnable:G,programLog:R,vertexShader:{log:j,prefix:w},fragmentShader:{log:tt,prefix:f}}}s.deleteShader(S),s.deleteShader(E),g=new _s(s,v),y=cm(s,v)}let g;this.getUniforms=function(){if(g===void 0)P(this);return g};let y;this.getAttributes=function(){if(y===void 0)P(this);return y};let D=n.rendererExtensionParallelShaderCompile===!1;return this.isReady=function(){if(D===!1)D=s.getProgramParameter(v,jp);return D},this.destroy=function(){i.releaseStatesOfProgram(this),s.deleteProgram(v),this.program=void 0},this.type=n.shaderType,this.name=n.shaderName,this.id=tm++,this.cacheKey=e,this.usedTimes=1,this.program=v,this.vertexShader=S,this.fragmentShader=E,this}var Mm=0;class ch{constructor(){this.shaderCache=new Map,this.materialCache=new Map}update(t,e,n){let i=this._getShaderCacheForMaterial(t);if(i.has(e)===!1)i.add(e),e.usedTimes++;if(i.has(n)===!1)i.add(n),n.usedTimes++;return this}remove(t){let e=this.materialCache.get(t);for(let n of e)if(n.usedTimes--,n.usedTimes===0)this.shaderCache.delete(n.code);return this.materialCache.delete(t),this}getVertexShaderStage(t){return this._getShaderStage(t.vertexShader)}getFragmentShaderStage(t){return this._getShaderStage(t.fragmentShader)}dispose(){this.shaderCache.clear(),this.materialCache.clear()}_getShaderCacheForMaterial(t){let e=this.materialCache,n=e.get(t);if(n===void 0)n=new Set,e.set(t,n);return n}_getShaderStage(t){let e=this.shaderCache,n=e.get(t);if(n===void 0)n=new hh(t),e.set(t,n);return n}}class hh{constructor(t){this.id=Mm++,this.code=t,this.usedTimes=0}}function Sm(t){return t===ci||t===or||t===lr}function bm(t,e,n,i,s,r){let a=new hr,o=new ch,l=new Set,c=[],p=new Map,h=i.logarithmicDepthBuffer,u={MeshDepthMaterial:"depth",MeshDistanceMaterial:"distance",MeshNormalMaterial:"normal",MeshBasicMaterial:"basic",MeshLambertMaterial:"lambert",MeshPhongMaterial:"phong",MeshToonMaterial:"toon",MeshStandardMaterial:"physical",MeshPhysicalMaterial:"physical",MeshMatcapMaterial:"matcap",LineBasicMaterial:"basic",LineDashedMaterial:"dashed",PointsMaterial:"points",ShadowMaterial:"shadow",SpriteMaterial:"sprite"};function d(g){if(l.add(g),g===0)return"uv";return`uv${g}`}function v(g,y,D,q,N,X){let rt=q.fog,R=N.geometry,j=g.isMeshStandardMaterial||g.isMeshLambertMaterial||g.isMeshPhongMaterial?q.environment:null,tt=g.isMeshStandardMaterial||g.isMeshLambertMaterial&&!g.envMap||g.isMeshPhongMaterial&&!g.envMap,G=e.get(g.envMap||j,tt),pt=u[g.type],J=i.precision;if(g.precision!==null){if(J=i.getMaxPrecision(g.precision),J!==g.precision)Wt("WebGLProgram.getParameters:",g.precision,"not supported, using",J,"instead.")}let st=R.morphAttributes.position||R.morphAttributes.normal||R.morphAttributes.color,k=st!==void 0?st.length:0,It=0;if(R.morphAttributes.position!==void 0)It=1;if(R.morphAttributes.normal!==void 0)It=2;if(R.morphAttributes.color!==void 0)It=3;let nt,ct,yt,Gt;if(pt){let At=An[pt];nt=At.vertexShader,ct=At.fragmentShader}else{nt=g.vertexShader,ct=g.fragmentShader;let At=o.getVertexShaderStage(g),ce=o.getFragmentShaderStage(g);o.update(g,At,ce),yt=At.id,Gt=ce.id}let zt=t.getRenderTarget(),Dt=t.state.buffers.depth.getReversed(),ue=N.isInstancedMesh===!0,ee=N.isBatchedMesh===!0,Jt=!!g.map,ve=!!g.matcap,ie=!!G,Me=ie&&G.isPMREMTexture===!0,ge=!!g.aoMap,Re=!!g.lightMap,Le=!!g.bumpMap&&g.wireframe===!1,_e=!!g.normalMap,be=!!g.displacementMap,Ue=!!g.emissiveMap,Fe=!!g.metalnessMap,Je=!!g.roughnessMap,U=g.anisotropy>0,Ye=g.clearcoat>0,de=g.diffuseRoughness>0,xe=g.dispersion>0,at=g.retroreflectivity>0,re=g.iridescence>0,kt=g.sheen>0,Xt=g.transmission>0,Oe=U&&!!g.anisotropyMap,b=Ye&&!!g.clearcoatMap,_=Ye&&!!g.clearcoatNormalMap,B=Ye&&!!g.clearcoatRoughnessMap,ht=de&&!!g.diffuseRoughnessMap,it=re&&!!g.iridescenceMap,mt=re&&!!g.iridescenceThicknessMap,Mt=kt&&!!g.sheenColorMap,Y=kt&&!!g.sheenRoughnessMap,et=!!g.specularMap,dt=!!g.specularColorMap,I=!!g.specularIntensityMap,Q=Xt&&!!g.transmissionMap,Z=Xt&&!!g.thicknessMap,ut=!!g.gradientMap,ft=!!g.alphaMap,St=g.alphaTest>0,Bt=!!g.alphaHash,Ft=!!g.extensions,Et=Sn;if(g.toneMapped){if(zt===null||zt.isXRRenderTarget===!0)Et=t.toneMapping}let Ot=Me?G.isCompressedCubeTexture?G.image[0].mipmaps:G.mipmaps:null,Vt={shaderID:pt,shaderType:g.type,shaderName:g.name,vertexShader:nt,fragmentShader:ct,defines:g.defines,customVertexShaderID:yt,customFragmentShaderID:Gt,isRawShaderMaterial:g.isRawShaderMaterial===!0,glslVersion:g.glslVersion,precision:J,batching:ee,batchingColor:ee&&N._colorsTexture!==null,instancing:ue,instancingColor:ue&&N.instanceColor!==null,instancingMorph:ue&&N.morphTexture!==null,outputColorSpace:zt===null?t.outputColorSpace:zt.isXRRenderTarget===!0?zt.texture.colorSpace:ae.workingColorSpace,alphaToCoverage:!!g.alphaToCoverage,map:Jt,matcap:ve,envMap:ie,envMapMode:ie&&G.mapping,envMapPMREM:Me,envMapMaxLod:Me?Ot.length-1:null,envMapSize:Me?Ot[0].width:null,aoMap:ge,lightMap:Re,bumpMap:Le,normalMap:_e,displacementMap:be,emissiveMap:Ue,normalMapObjectSpace:_e&&g.normalMapType===uc,normalMapTangentSpace:_e&&g.normalMapType===Qa,packedNormalMap:_e&&g.normalMapType===Qa&&Sm(g.normalMap.format),metalnessMap:Fe,roughnessMap:Je,anisotropy:U,anisotropyMap:Oe,clearcoat:Ye,clearcoatMap:b,clearcoatNormalMap:_,clearcoatRoughnessMap:B,diffuseRoughness:de,diffuseRoughnessMap:ht,dispersion:xe,retroreflection:at,iridescence:re,iridescenceMap:it,iridescenceThicknessMap:mt,sheen:kt,sheenColorMap:Mt,sheenRoughnessMap:Y,specularMap:et,specularColorMap:dt,specularIntensityMap:I,transmission:Xt,transmissionMap:Q,thicknessMap:Z,gradientMap:ut,opaque:g.transparent===!1&&g.blending===oi&&g.alphaToCoverage===!1,alphaMap:ft,alphaTest:St,alphaHash:Bt,combine:g.combine,mapUv:Jt&&d(g.map.channel),aoMapUv:ge&&d(g.aoMap.channel),lightMapUv:Re&&d(g.lightMap.channel),bumpMapUv:Le&&d(g.bumpMap.channel),normalMapUv:_e&&d(g.normalMap.channel),displacementMapUv:be&&d(g.displacementMap.channel),emissiveMapUv:Ue&&d(g.emissiveMap.channel),metalnessMapUv:Fe&&d(g.metalnessMap.channel),roughnessMapUv:Je&&d(g.roughnessMap.channel),anisotropyMapUv:Oe&&d(g.anisotropyMap.channel),clearcoatMapUv:b&&d(g.clearcoatMap.channel),clearcoatNormalMapUv:_&&d(g.clearcoatNormalMap.channel),clearcoatRoughnessMapUv:B&&d(g.clearcoatRoughnessMap.channel),diffuseRoughnessMapUv:ht&&d(g.diffuseRoughnessMap.channel),iridescenceMapUv:it&&d(g.iridescenceMap.channel),iridescenceThicknessMapUv:mt&&d(g.iridescenceThicknessMap.channel),sheenColorMapUv:Mt&&d(g.sheenColorMap.channel),sheenRoughnessMapUv:Y&&d(g.sheenRoughnessMap.channel),specularMapUv:et&&d(g.specularMap.channel),specularColorMapUv:dt&&d(g.specularColorMap.channel),specularIntensityMapUv:I&&d(g.specularIntensityMap.channel),transmissionMapUv:Q&&d(g.transmissionMap.channel),thicknessMapUv:Z&&d(g.thicknessMap.channel),alphaMapUv:ft&&d(g.alphaMap.channel),vertexTangents:!!R.attributes.tangent&&(_e||U),vertexNormals:!!R.attributes.normal,vertexColors:g.vertexColors,vertexAlphas:g.vertexColors===!0&&!!R.attributes.color&&R.attributes.color.itemSize===4,pointsUvs:N.isPoints===!0&&!!R.attributes.uv&&(Jt||ft),fog:!!rt,useFog:g.fog===!0,fogExp2:!!rt&&rt.isFogExp2,flatShading:g.wireframe===!1&&(g.flatShading===!0||R.attributes.normal===void 0&&_e===!1&&(g.isMeshLambertMaterial||g.isMeshPhongMaterial||g.isMeshStandardMaterial||g.isMeshPhysicalMaterial)),sizeAttenuation:g.sizeAttenuation===!0,logarithmicDepthBuffer:h,reversedDepthBuffer:Dt,skinning:N.isSkinnedMesh===!0,hasPositionAttribute:R.attributes.position!==void 0,morphTargets:R.morphAttributes.position!==void 0,morphNormals:R.morphAttributes.normal!==void 0,morphColors:R.morphAttributes.color!==void 0,morphTargetsCount:k,morphTextureStride:It,numSunLights:y.sun.length,numDirLights:y.directional.length,numPointLights:y.point.length,numSpotLights:y.spot.length,numSpotLightMaps:y.spotLightMap.length,numRectAreaLights:y.rectArea.length,numHemiLights:y.hemi.length,numSunLightShadows:y.sunShadowMap.length,numDirLightShadows:y.directionalShadowMap.length,numPointLightShadows:y.pointShadowMap.length,numSpotLightShadows:y.spotShadowMap.length,numSpotLightShadowsWithMaps:y.numSpotLightShadowsWithMaps,numLightProbes:y.numLightProbes,numLightProbeGrids:X.filter((At)=>At.texture!==null).length,numClippingPlanes:r.numPlanes,numClipIntersection:r.numIntersection,dithering:g.dithering,shadowMapEnabled:t.shadowMap.enabled&&D.length>0,shadowMapType:t.shadowMap.type,toneMapping:Et,decodeVideoTexture:Jt&&g.map.isVideoTexture===!0&&ae.getTransfer(g.map.colorSpace)===ye,decodeVideoTextureEmissive:Ue&&g.emissiveMap.isVideoTexture===!0&&ae.getTransfer(g.emissiveMap.colorSpace)===ye,premultipliedAlpha:g.premultipliedAlpha,doubleSided:g.side===rn,flipSided:g.side===Qe,useDepthPacking:g.depthPacking>=0,depthPacking:g.depthPacking||0,index0AttributeName:g.index0AttributeName,extensionClipCullDistance:Ft&&g.extensions.clipCullDistance===!0&&n.has("WEBGL_clip_cull_distance"),extensionMultiDraw:(Ft&&g.extensions.multiDraw===!0||ee)&&n.has("WEBGL_multi_draw"),rendererExtensionParallelShaderCompile:n.has("KHR_parallel_shader_compile"),customProgramCacheKey:g.customProgramCacheKey()};return Vt.vertexUv1s=l.has(1),Vt.vertexUv2s=l.has(2),Vt.vertexUv3s=l.has(3),l.clear(),Vt}function w(g){let y=[];if(g.shaderID)y.push(g.shaderID);else y.push(g.customVertexShaderID),y.push(g.customFragmentShaderID);if(g.defines!==void 0)for(let D in g.defines)y.push(D),y.push(g.defines[D]);if(g.isRawShaderMaterial===!1)f(y,g),m(y,g),y.push(t.outputColorSpace);return y.push(g.customProgramCacheKey),y.join()}function f(g,y){g.push(y.precision),g.push(y.outputColorSpace),g.push(y.envMapMode),g.push(y.envMapPMREM),g.push(y.envMapMaxLod),g.push(y.envMapSize),g.push(y.mapUv),g.push(y.alphaMapUv),g.push(y.lightMapUv),g.push(y.aoMapUv),g.push(y.bumpMapUv),g.push(y.normalMapUv),g.push(y.displacementMapUv),g.push(y.emissiveMapUv),g.push(y.metalnessMapUv),g.push(y.roughnessMapUv),g.push(y.anisotropyMapUv),g.push(y.clearcoatMapUv),g.push(y.clearcoatNormalMapUv),g.push(y.clearcoatRoughnessMapUv),g.push(y.diffuseRoughnessMapUv),g.push(y.iridescenceMapUv),g.push(y.iridescenceThicknessMapUv),g.push(y.sheenColorMapUv),g.push(y.sheenRoughnessMapUv),g.push(y.specularMapUv),g.push(y.specularColorMapUv),g.push(y.specularIntensityMapUv),g.push(y.transmissionMapUv),g.push(y.thicknessMapUv),g.push(y.combine),g.push(y.fogExp2),g.push(y.sizeAttenuation),g.push(y.morphTargetsCount),g.push(y.numSunLights),g.push(y.numDirLights),g.push(y.numPointLights),g.push(y.numSpotLights),g.push(y.numSpotLightMaps),g.push(y.numHemiLights),g.push(y.numRectAreaLights),g.push(y.numSunLightShadows),g.push(y.numDirLightShadows),g.push(y.numPointLightShadows),g.push(y.numSpotLightShadows),g.push(y.numSpotLightShadowsWithMaps),g.push(y.numLightProbes),g.push(y.numLightProbeGrids),g.push(y.shadowMapType),g.push(y.toneMapping),g.push(y.numClippingPlanes),g.push(y.numClipIntersection),g.push(y.depthPacking)}function m(g,y){if(a.disableAll(),y.instancing)a.enable(0);if(y.instancingColor)a.enable(1);if(y.instancingMorph)a.enable(2);if(y.matcap)a.enable(3);if(y.envMap)a.enable(4);if(y.normalMapObjectSpace)a.enable(5);if(y.normalMapTangentSpace)a.enable(6);if(y.clearcoat)a.enable(7);if(y.iridescence)a.enable(8);if(y.alphaTest)a.enable(9);if(y.vertexColors)a.enable(10);if(y.vertexAlphas)a.enable(11);if(y.vertexUv1s)a.enable(12);if(y.vertexUv2s)a.enable(13);if(y.vertexUv3s)a.enable(14);if(y.vertexTangents)a.enable(15);if(y.anisotropy)a.enable(16);if(y.alphaHash)a.enable(17);if(y.batching)a.enable(18);if(y.dispersion)a.enable(19);if(y.retroreflection)a.enable(24);if(y.diffuseRoughness)a.enable(25);if(y.batchingColor)a.enable(20);if(y.gradientMap)a.enable(21);if(y.packedNormalMap)a.enable(22);if(y.vertexNormals)a.enable(23);if(g.push(a.mask),a.disableAll(),y.fog)a.enable(0);if(y.useFog)a.enable(1);if(y.flatShading)a.enable(2);if(y.logarithmicDepthBuffer)a.enable(3);if(y.reversedDepthBuffer)a.enable(4);if(y.skinning)a.enable(5);if(y.morphTargets)a.enable(6);if(y.morphNormals)a.enable(7);if(y.morphColors)a.enable(8);if(y.premultipliedAlpha)a.enable(9);if(y.shadowMapEnabled)a.enable(10);if(y.doubleSided)a.enable(11);if(y.flipSided)a.enable(12);if(y.useDepthPacking)a.enable(13);if(y.dithering)a.enable(14);if(y.transmission)a.enable(15);if(y.sheen)a.enable(16);if(y.opaque)a.enable(17);if(y.pointsUvs)a.enable(18);if(y.decodeVideoTexture)a.enable(19);if(y.decodeVideoTextureEmissive)a.enable(20);if(y.alphaToCoverage)a.enable(21);if(y.numLightProbeGrids>0)a.enable(22);if(y.hasPositionAttribute)a.enable(23);g.push(a.mask)}function M(g){let y=u[g.type],D;if(y){let q=An[y];D=wc.clone(q.uniforms)}else D=g.uniforms;return D}function T(g,y){let D=p.get(y);if(D!==void 0)++D.usedTimes;else D=new ym(t,y,g,s),c.push(D),p.set(y,D);return D}function S(g){if(--g.usedTimes===0){let y=c.indexOf(g);c[y]=c[c.length-1],c.pop(),p.delete(g.cacheKey),g.destroy()}}function E(g){o.remove(g)}function P(){o.dispose()}return{getParameters:v,getProgramCacheKey:w,getUniforms:M,acquireProgram:T,releaseProgram:S,releaseShaderCache:E,programs:c,dispose:P}}function wm(){let t=new WeakMap;function e(a){return t.has(a)}function n(a){let o=t.get(a);if(o===void 0)o={},t.set(a,o);return o}function i(a){t.delete(a)}function s(a,o,l){t.get(a)[o]=l}function r(){t=new WeakMap}return{has:e,get:n,remove:i,update:s,dispose:r}}function Am(t,e){if(t.groupOrder!==e.groupOrder)return t.groupOrder-e.groupOrder;else if(t.renderOrder!==e.renderOrder)return t.renderOrder-e.renderOrder;else if(t.material.id!==e.material.id)return t.material.id-e.material.id;else if(t.materialVariant!==e.materialVariant)return t.materialVariant-e.materialVariant;else if(t.z!==e.z)return t.z-e.z;else return t.id-e.id}function qc(t,e){if(t.groupOrder!==e.groupOrder)return t.groupOrder-e.groupOrder;else if(t.renderOrder!==e.renderOrder)return t.renderOrder-e.renderOrder;else if(t.z!==e.z)return e.z-t.z;else return t.id-e.id}function Yc(){let t=[],e=0,n=[],i=[],s=[];function r(){e=0,n.length=0,i.length=0,s.length=0}function a(u){let d=0;if(u.isInstancedMesh)d+=2;if(u.isSkinnedMesh)d+=1;return d}function o(u,d,v,w,f,m){let M=t[e];if(M===void 0)M={id:u.id,object:u,geometry:d,material:v,materialVariant:a(u),groupOrder:w,renderOrder:u.renderOrder,z:f,group:m},t[e]=M;else M.id=u.id,M.object=u,M.geometry=d,M.material=v,M.materialVariant=a(u),M.groupOrder=w,M.renderOrder=u.renderOrder,M.z=f,M.group=m;return e++,M}function l(u,d,v,w,f,m,M){if(M.reversedDepth===!0)f=-f;let T=o(u,d,v,w,f,m);if(v.transmission>0)i.push(T);else if(v.transparent===!0)s.push(T);else n.push(T)}function c(u,d,v,w,f,m){let M=o(u,d,v,w,f,m);if(v.transmission>0)i.unshift(M);else if(v.transparent===!0)s.unshift(M);else n.unshift(M)}function p(u,d){if(n.length>1)n.sort(u||Am);if(i.length>1)i.sort(d||qc);if(s.length>1)s.sort(d||qc)}function h(){for(let u=e,d=t.length;u<d;u++){let v=t[u];if(v.id===null)break;v.id=null,v.object=null,v.geometry=null,v.material=null,v.group=null}}return{opaque:n,transmissive:i,transparent:s,init:r,push:l,unshift:c,finish:h,sort:p}}function Tm(){let t=new WeakMap;function e(i,s){let r=t.get(i),a;if(r===void 0)a=new Yc,t.set(i,[a]);else if(s>=r.length)a=new Yc,r.push(a);else a=r[s];return a}function n(){t=new WeakMap}return{get:e,dispose:n}}function Em(){let t={};return{get:function(e){if(t[e.id]!==void 0)return t[e.id];let n;switch(e.type){case"SunLight":case"DirectionalLight":n={direction:new W,color:new le};break;case"SpotLight":n={position:new W,direction:new W,color:new le,distance:0,coneCos:0,penumbraCos:0,decay:0};break;case"PointLight":n={position:new W,color:new le,distance:0,decay:0};break;case"HemisphereLight":n={direction:new W,skyColor:new le,groundColor:new le};break;case"RectAreaLight":n={color:new le,position:new W,halfWidth:new W,halfHeight:new W};break}return t[e.id]=n,n}}}function Rm(){let t={};return{get:function(e){if(t[e.id]!==void 0)return t[e.id];let n;switch(e.type){case"SunLight":case"DirectionalLight":n={shadowIntensity:1,shadowBias:0,shadowNormalBias:0,shadowRadius:1,shadowMapSize:new fe};break;case"SpotLight":n={shadowIntensity:1,shadowBias:0,shadowNormalBias:0,shadowRadius:1,shadowMapSize:new fe};break;case"PointLight":n={shadowIntensity:1,shadowBias:0,shadowNormalBias:0,shadowRadius:1,shadowMapSize:new fe,shadowCameraNear:1,shadowCameraFar:1000};break}return t[e.id]=n,n}}}var Cm=0;function Im(t,e){return(e.castShadow?2:0)-(t.castShadow?2:0)+(e.map?1:0)-(t.map?1:0)}function Pm(t){let e=new Em,n=Rm(),i={version:0,hash:{sunLength:-1,directionalLength:-1,pointLength:-1,spotLength:-1,rectAreaLength:-1,hemiLength:-1,numSunShadows:-1,numDirectionalShadows:-1,numPointShadows:-1,numSpotShadows:-1,numSpotMaps:-1,numLightProbes:-1},ambient:[0,0,0],probe:[],sun:[],sunShadow:[],sunShadowMap:[],sunShadowMatrix:[],sunShadowCascade:[],directional:[],directionalShadow:[],directionalShadowMap:[],directionalShadowMatrix:[],spot:[],spotLightMap:[],spotShadow:[],spotShadowMap:[],spotLightMatrix:[],rectArea:[],rectAreaLTC1:null,rectAreaLTC2:null,point:[],pointShadow:[],pointShadowMap:[],pointShadowMatrix:[],hemi:[],numSpotLightShadowsWithMaps:0,numLightProbes:0};for(let c=0;c<9;c++)i.probe.push(new W);let s=new W,r=new Se,a=new Se;function o(c){let p=0,h=0,u=0;for(let X=0;X<9;X++)i.probe[X].set(0,0,0);let d=0,v=0,w=0,f=0,m=0,M=0,T=0,S=0,E=0,P=0,g=0,y=0,D=0,q=0;c.sort(Im);for(let X=0,rt=c.length;X<rt;X++){let R=c[X],{color:j,intensity:tt,distance:G}=R,pt=null;if(R.shadow&&R.shadow.map)if(R.shadow.map.texture.format===ci)pt=R.shadow.map.texture;else pt=R.shadow.map.depthTexture||R.shadow.map.texture;if(R.isAmbientLight)p+=j.r*tt,h+=j.g*tt,u+=j.b*tt;else if(R.isLightProbe){for(let J=0;J<9;J++)i.probe[J].addScaledVector(R.sh.coefficients[J],tt);q++}else if(R.isSunLight){let J=e.get(R);if(J.color.copy(R.color).multiplyScalar(R.intensity),R.castShadow){let st=R.shadow,k=n.get(R);k.shadowIntensity=st.intensity,k.shadowBias=st.bias,k.shadowNormalBias=st.normalBias,k.shadowRadius=st.radius,k.shadowMapSize.copy(st.mapSize).multiply(st.getFrameExtents()),i.sunShadow[v]=k,i.sunShadowMap[v]=pt;let It=st.getViewportCount();for(let nt=0;nt<It;nt++)i.sunShadowMatrix[w+nt]=st.getMatrix(nt),i.sunShadowCascade[w+nt]=st._cascadeData[nt];w+=It,v++}i.sun[d]=J,d++}else if(R.isDirectionalLight){let J=e.get(R);if(J.color.copy(R.color).multiplyScalar(R.intensity),R.castShadow){let st=R.shadow,k=n.get(R);k.shadowIntensity=st.intensity,k.shadowBias=st.bias,k.shadowNormalBias=st.normalBias,k.shadowRadius=st.radius,k.shadowMapSize=st.mapSize,i.directionalShadow[f]=k,i.directionalShadowMap[f]=pt,i.directionalShadowMatrix[f]=R.shadow.matrix,E++}i.directional[f]=J,f++}else if(R.isSpotLight){let J=e.get(R);J.position.setFromMatrixPosition(R.matrixWorld),J.color.copy(j).multiplyScalar(tt),J.distance=G,J.coneCos=Math.cos(R.angle),J.penumbraCos=Math.cos(R.angle*(1-R.penumbra)),J.decay=R.decay,i.spot[M]=J;let st=R.shadow;if(R.map){if(i.spotLightMap[y]=R.map,y++,st.updateMatrices(R),R.castShadow)D++}if(i.spotLightMatrix[M]=st.matrix,R.castShadow){let k=n.get(R);k.shadowIntensity=st.intensity,k.shadowBias=st.bias,k.shadowNormalBias=st.normalBias,k.shadowRadius=st.radius,k.shadowMapSize=st.mapSize,i.spotShadow[M]=k,i.spotShadowMap[M]=pt,g++}M++}else if(R.isRectAreaLight){let J=e.get(R);J.color.copy(j).multiplyScalar(tt),J.halfWidth.set(R.width*0.5,0,0),J.halfHeight.set(0,R.height*0.5,0),i.rectArea[T]=J,T++}else if(R.isPointLight){let J=e.get(R);if(J.color.copy(R.color).multiplyScalar(R.intensity),J.distance=R.distance,J.decay=R.decay,R.castShadow){let st=R.shadow,k=n.get(R);k.shadowIntensity=st.intensity,k.shadowBias=st.bias,k.shadowNormalBias=st.normalBias,k.shadowRadius=st.radius,k.shadowMapSize=st.mapSize,k.shadowCameraNear=st.camera.near,k.shadowCameraFar=st.camera.far,i.pointShadow[m]=k,i.pointShadowMap[m]=pt,i.pointShadowMatrix[m]=R.shadow.matrix,P++}i.point[m]=J,m++}else if(R.isHemisphereLight){let J=e.get(R);J.skyColor.copy(R.color).multiplyScalar(tt),J.groundColor.copy(R.groundColor).multiplyScalar(tt),i.hemi[S]=J,S++}}if(T>0)if(t.has("OES_texture_float_linear")===!0)i.rectAreaLTC1=Rt.LTC_FLOAT_1,i.rectAreaLTC2=Rt.LTC_FLOAT_2;else i.rectAreaLTC1=Rt.LTC_HALF_1,i.rectAreaLTC2=Rt.LTC_HALF_2;i.ambient[0]=p,i.ambient[1]=h,i.ambient[2]=u;let N=i.hash;if(N.sunLength!==d||N.directionalLength!==f||N.pointLength!==m||N.spotLength!==M||N.rectAreaLength!==T||N.hemiLength!==S||N.numSunShadows!==v||N.numDirectionalShadows!==E||N.numPointShadows!==P||N.numSpotShadows!==g||N.numSpotMaps!==y||N.numLightProbes!==q)i.sun.length=d,i.directional.length=f,i.spot.length=M,i.rectArea.length=T,i.point.length=m,i.hemi.length=S,i.sunShadow.length=v,i.sunShadowMap.length=v,i.sunShadowMatrix.length=w,i.sunShadowCascade.length=w,i.directionalShadow.length=E,i.directionalShadowMap.length=E,i.directionalShadowMatrix.length=E,i.pointShadow.length=P,i.pointShadowMap.length=P,i.pointShadowMatrix.length=P,i.spotShadow.length=g,i.spotShadowMap.length=g,i.spotLightMatrix.length=g+y-D,i.spotLightMap.length=y,i.numSpotLightShadowsWithMaps=D,i.numLightProbes=q,N.sunLength=d,N.directionalLength=f,N.pointLength=m,N.spotLength=M,N.rectAreaLength=T,N.hemiLength=S,N.numSunShadows=v,N.numDirectionalShadows=E,N.numPointShadows=P,N.numSpotShadows=g,N.numSpotMaps=y,N.numLightProbes=q,i.version=Cm++}function l(c,p){let h=0,u=0,d=0,v=0,w=0,f=0,m=p.matrixWorldInverse;for(let M=0,T=c.length;M<T;M++){let S=c[M];if(S.isSunLight){let E=i.sun[h];E.direction.setFromMatrixPosition(S.matrixWorld),E.direction.transformDirection(m),h++}else if(S.isDirectionalLight){let E=i.directional[u];E.direction.setFromMatrixPosition(S.matrixWorld),s.setFromMatrixPosition(S.target.matrixWorld),E.direction.sub(s),E.direction.transformDirection(m),u++}else if(S.isSpotLight){let E=i.spot[v];E.position.setFromMatrixPosition(S.matrixWorld),E.position.applyMatrix4(m),E.direction.setFromMatrixPosition(S.matrixWorld),s.setFromMatrixPosition(S.target.matrixWorld),E.direction.sub(s),E.direction.transformDirection(m),v++}else if(S.isRectAreaLight){let E=i.rectArea[w];E.position.setFromMatrixPosition(S.matrixWorld),E.position.applyMatrix4(m),a.identity(),r.copy(S.matrixWorld),r.premultiply(m),a.extractRotation(r),E.halfWidth.set(S.width*0.5,0,0),E.halfHeight.set(0,S.height*0.5,0),E.halfWidth.applyMatrix4(a),E.halfHeight.applyMatrix4(a),w++}else if(S.isPointLight){let E=i.point[d];E.position.setFromMatrixPosition(S.matrixWorld),E.position.applyMatrix4(m),d++}else if(S.isHemisphereLight){let E=i.hemi[f];E.direction.setFromMatrixPosition(S.matrixWorld),E.direction.transformDirection(m),f++}}}return{setup:o,setupView:l,state:i}}function Zc(t){let e=new Pm(t),n=[],i=[],s=[];function r(h){p.camera=h,n.length=0,i.length=0,s.length=0}function a(h){if(h.isLightProbeGrid)s.push(h);else n.push(h)}function o(h){i.push(h)}function l(){e.setup(n),s.sort((h,u)=>h.priority-u.priority||h.id-u.id)}function c(h){e.setupView(n,h)}let p={lightsArray:n,shadowsArray:i,lightProbeGridArray:s,camera:null,lights:e,transmissionRenderTarget:{},textureUnits:0};return{init:r,state:p,setupLights:l,setupLightsView:c,pushLight:a,pushShadow:o}}function Lm(t){let e=new WeakMap;function n(s,r=0){let a=e.get(s),o;if(a===void 0)o=new Zc(t),e.set(s,[o]);else if(r>=a.length)o=new Zc(t),a.push(o);else o=a[r];return o}function i(){e=new WeakMap}return{get:n,dispose:i}}var Nm=`void main() {
	gl_Position = vec4( position, 1.0 );
}`,Dm=`uniform sampler2D shadow_pass;
uniform vec2 resolution;
uniform float radius;
void main() {
	const float samples = float( VSM_SAMPLES );
	float mean = 0.0;
	float squared_mean = 0.0;
	float uvStride = samples <= 1.0 ? 0.0 : 2.0 / ( samples - 1.0 );
	float uvStart = samples <= 1.0 ? 0.0 : - 1.0;
	for ( float i = 0.0; i < samples; i ++ ) {
		float uvOffset = uvStart + i * uvStride;
		#ifdef HORIZONTAL_PASS
			vec2 distribution = texture2D( shadow_pass, ( gl_FragCoord.xy + vec2( uvOffset, 0.0 ) * radius ) / resolution ).rg;
			mean += distribution.x;
			squared_mean += distribution.y * distribution.y + distribution.x * distribution.x;
		#else
			float depth = texture2D( shadow_pass, ( gl_FragCoord.xy + vec2( 0.0, uvOffset ) * radius ) / resolution ).r;
			mean += depth;
			squared_mean += depth * depth;
		#endif
	}
	mean = mean / samples;
	squared_mean = squared_mean / samples;
	float std_dev = sqrt( max( 0.0, squared_mean - mean * mean ) );
	gl_FragColor = vec4( mean, std_dev, 0.0, 1.0 );
}`,Um=[new W(1,0,0),new W(-1,0,0),new W(0,1,0),new W(0,-1,0),new W(0,0,1),new W(0,0,-1)],Fm=[new W(0,-1,0),new W(0,-1,0),new W(0,0,1),new W(0,0,-1),new W(0,-1,0),new W(0,-1,0)],Jc=new Se,ms=new W,Oo=new W;function Om(t,e,n){let i=new Fi,s=new fe,r=new fe,a=new Ae,o=new vo,l=new yo,c={},p=n.maxTextureSize,h={[Bi]:Qe,[Qe]:Bi,[rn]:rn},u=new vn({defines:{VSM_SAMPLES:8},uniforms:{shadow_pass:{value:null},resolution:{value:new fe},radius:{value:4}},vertexShader:Nm,fragmentShader:Dm}),d=u.clone();d.defines.HORIZONTAL_PASS=1;let v=new Ee;v.setAttribute("position",new Ke(new Float32Array([-1,-1,0.5,3,-1,0.5,-1,3,0.5]),3));let w=new hn(v,u),f=this;this.enabled=!1,this.autoUpdate=!0,this.needsUpdate=!1,this.type=is;let m=this.type;this.render=function(P,g,y){if(f.enabled===!1)return;if(f.autoUpdate===!1&&f.needsUpdate===!1)return;if(P.length===0)return;if(this.type===wl)Wt("WebGLShadowMap: PCFSoftShadowMap has been removed. Using PCFShadowMap instead."),this.type=is;let D=t.getRenderTarget(),q=t.getActiveCubeFace(),N=t.getActiveMipmapLevel(),X=t.state;if(X.setBlending(ai),X.buffers.depth.getReversed()===!0)X.buffers.color.setClear(0,0,0,0);else X.buffers.color.setClear(1,1,1,1);X.buffers.depth.setTest(!0),X.setScissorTest(!1);let rt=m!==this.type;if(rt)g.traverse(function(R){if(R.material)if(Array.isArray(R.material))R.material.forEach((j)=>j.needsUpdate=!0);else R.material.needsUpdate=!0});for(let R=0,j=P.length;R<j;R++){let tt=P[R],G=tt.shadow;if(G===void 0){Wt("WebGLShadowMap:",tt,"has no shadow.");continue}if(G.autoUpdate===!1&&G.needsUpdate===!1)continue;s.copy(G.mapSize);let pt=G.getFrameExtents();if(s.multiply(pt),r.copy(G.mapSize),s.x>p||s.y>p){if(s.x>p)r.x=Math.floor(p/pt.x),s.x=r.x*pt.x,G.mapSize.x=r.x;if(s.y>p)r.y=Math.floor(p/pt.y),s.y=r.y*pt.y,G.mapSize.y=r.y}let J=t.state.buffers.depth.getReversed();if(G.camera._reversedDepth=J,G.map===null||rt===!0){if(G.map!==null){if(G.map.depthTexture!==null)G.map.depthTexture.dispose(),G.map.depthTexture=null;G.map.dispose()}if(this.type===Oi){if(tt.isPointLight){Wt("WebGLShadowMap: VSM shadow maps are not supported for PointLights. Use PCF or BasicShadowMap instead.");continue}G.map=new cn(s.x,s.y,{format:ci,type:bn,minFilter:ln,magFilter:ln,generateMipmaps:!1}),G.map.texture.name=tt.name+".shadowMap",G.map.depthTexture=new Kn(s.x,s.y,Un),G.map.depthTexture.name=tt.name+".shadowMapDepth",G.map.depthTexture.format=Gi,G.map.depthTexture.compareFunction=null,G.map.depthTexture.minFilter=qn,G.map.depthTexture.magFilter=qn}else{if(tt.isPointLight)G.map=new Ar(s.x),G.map.depthTexture=new xr(s.x,Zn);else G.map=new cn(s.x,s.y),G.map.depthTexture=new Kn(s.x,s.y,Zn);if(G.map.depthTexture.name=tt.name+".shadowMap",G.map.depthTexture.format=Gi,this.type===is)G.map.depthTexture.compareFunction=J?no:eo,G.map.depthTexture.minFilter=ln,G.map.depthTexture.magFilter=ln;else G.map.depthTexture.compareFunction=null,G.map.depthTexture.minFilter=qn,G.map.depthTexture.magFilter=qn}G.camera.updateProjectionMatrix()}if(G.map.isWebGLCubeRenderTarget!==!0&&(G.map.width!==s.x||G.map.height!==s.y))G.map.setSize(s.x,s.y);let st=G.map.isWebGLCubeRenderTarget?6:G.getViewportCount();if(tt.isPointLight!==!0)G.updateMatrices(tt,y);let k=(G.camera.layers.mask&4294967294)!==0?G.camera.layers:y.layers;for(let It=0;It<st;It++){let nt=G.getCamera(It);if(tt.isPointLight){let{camera:ct,matrix:yt}=G,Gt=tt.distance||ct.far;if(Gt!==ct.far)ct.far=Gt,ct.updateProjectionMatrix();ms.setFromMatrixPosition(tt.matrixWorld),ct.position.copy(ms),Oo.copy(ct.position),Oo.add(Um[It]),ct.up.copy(Fm[It]),ct.lookAt(Oo),ct.updateMatrixWorld(),yt.makeTranslation(-ms.x,-ms.y,-ms.z),Jc.multiplyMatrices(ct.projectionMatrix,ct.matrixWorldInverse),G._frustum.setFromProjectionMatrix(Jc,ct.coordinateSystem,ct.reversedDepth)}if(G.map.isWebGLCubeRenderTarget)t.setRenderTarget(G.map,It),t.clear();else{if(It===0)t.setRenderTarget(G.map),t.clear();let ct=G.getViewport(It);a.set(r.x*ct.x,r.y*ct.y,r.x*ct.z,r.y*ct.w),X.viewport(a)}i=G.getFrustum(It),S(g,y,nt,tt,k,this.type)}if(G.isPointLightShadow!==!0&&this.type===Oi)M(G,y);G.needsUpdate=!1}m=this.type,f.needsUpdate=!1,t.setRenderTarget(D,q,N)};function M(P,g){let y=e.update(w);if(u.defines.VSM_SAMPLES!==P.blurSamples)u.defines.VSM_SAMPLES=P.blurSamples,d.defines.VSM_SAMPLES=P.blurSamples,u.needsUpdate=!0,d.needsUpdate=!0;if(P.mapPass===null)P.mapPass=new cn(s.x,s.y,{format:ci,type:bn});else if(P.mapPass.width!==P.map.width||P.mapPass.height!==P.map.height)P.mapPass.setSize(P.map.width,P.map.height);u.uniforms.shadow_pass.value=P.map.depthTexture,u.uniforms.resolution.value.set(P.map.width,P.map.height),u.uniforms.radius.value=P.radius,t.setRenderTarget(P.mapPass),t.clear(),t.renderBufferDirect(g,null,y,u,w,null),d.uniforms.shadow_pass.value=P.mapPass.texture,d.uniforms.resolution.value.set(P.map.width,P.map.height),d.uniforms.radius.value=P.radius,t.setRenderTarget(P.map),t.clear(),t.renderBufferDirect(g,null,y,d,w,null)}function T(P,g,y,D){let q=null,N=y.isPointLight===!0?P.customDistanceMaterial:P.customDepthMaterial;if(N!==void 0)q=N;else if(q=y.isPointLight===!0?l:o,t.localClippingEnabled&&g.clipShadows===!0&&Array.isArray(g.clippingPlanes)&&g.clippingPlanes.length!==0||g.displacementMap&&g.displacementScale!==0||g.alphaMap&&g.alphaTest>0||g.map&&g.alphaTest>0||g.alphaToCoverage===!0){let X=q.uuid,rt=g.uuid,R=c[X];if(R===void 0)R={},c[X]=R;let j=R[rt];if(j===void 0)j=q.clone(),R[rt]=j,g.addEventListener("dispose",E);q=j}if(q.visible=g.visible,q.wireframe=g.wireframe,D===Oi)q.side=g.shadowSide!==null?g.shadowSide:g.side;else q.side=g.shadowSide!==null?g.shadowSide:h[g.side];if(q.alphaMap=g.alphaMap,q.alphaTest=g.alphaToCoverage===!0?0.5:g.alphaTest,q.map=g.map,q.clipShadows=g.clipShadows,q.clippingPlanes=g.clippingPlanes,q.clipIntersection=g.clipIntersection,q.displacementMap=g.displacementMap,q.displacementScale=g.displacementScale,q.displacementBias=g.displacementBias,q.wireframeLinewidth=g.wireframeLinewidth,q.linewidth=g.linewidth,y.isPointLight===!0&&q.isMeshDistanceMaterial===!0){let X=t.properties.get(q);X.light=y}return q}function S(P,g,y,D,q,N){if(P.visible===!1)return;if(P.layers.test(q)&&(P.isMesh||P.isLine||P.isPoints)){if((P.castShadow||P.receiveShadow&&N===Oi)&&(!P.frustumCulled||P.intersectsFrustum(i))){P.modelViewMatrix.multiplyMatrices(y.matrixWorldInverse,P.matrixWorld);let R=e.update(P),j=P.material;if(Array.isArray(j)){let tt=R.groups;for(let G=0,pt=tt.length;G<pt;G++){let J=tt[G],st=j[J.materialIndex];if(st&&st.visible){let k=T(P,st,D,N);P.onBeforeShadow(t,P,g,y,R,k,J),t.renderBufferDirect(y,null,R,k,P,J),P.onAfterShadow(t,P,g,y,R,k,J)}}}else if(j.visible){let tt=T(P,j,D,N);P.onBeforeShadow(t,P,g,y,R,tt,null),t.renderBufferDirect(y,null,R,tt,P,null),P.onAfterShadow(t,P,g,y,R,tt,null)}}}let rt=P.children;for(let R=0,j=rt.length;R<j;R++)S(rt[R],g,y,D,q,N)}function E(P){P.target.removeEventListener("dispose",E);for(let y in c){let D=c[y],q=P.target.uuid;if(q in D)D[q].dispose(),delete D[q]}}}function Bm(t,e){function n(){let I=!1,Q=new Ae,Z=null,ut=new Ae(0,0,0,0);return{setMask:function(ft){if(Z!==ft&&!I)t.colorMask(ft,ft,ft,ft),Z=ft},setLocked:function(ft){I=ft},setClear:function(ft,St,Bt,Ft,Et){if(Et===!0)ft*=Ft,St*=Ft,Bt*=Ft;if(Q.set(ft,St,Bt,Ft),ut.equals(Q)===!1)t.clearColor(ft,St,Bt,Ft),ut.copy(Q)},reset:function(){I=!1,Z=null,ut.set(-1,0,0,0)}}}function i(){let I=!1,Q=!1,Z=null,ut=null,ft=null;return{setReversed:function(St){if(Q!==St){let Bt=e.get("EXT_clip_control");if(St)Bt.clipControlEXT(Bt.LOWER_LEFT_EXT,Bt.ZERO_TO_ONE_EXT);else Bt.clipControlEXT(Bt.LOWER_LEFT_EXT,Bt.NEGATIVE_ONE_TO_ONE_EXT);Q=St;let Ft=ft;ft=null,this.setClear(Ft)}},getReversed:function(){return Q},setTest:function(St){if(St)Dt(t.DEPTH_TEST);else ue(t.DEPTH_TEST)},setMask:function(St){if(Z!==St&&!I)t.depthMask(St),Z=St},setFunc:function(St){if(Q)St=Mc[St];if(ut!==St){switch(St){case Xl:t.depthFunc(t.NEVER);break;case ql:t.depthFunc(t.ALWAYS);break;case Yl:t.depthFunc(t.LESS);break;case la:t.depthFunc(t.LEQUAL);break;case Zl:t.depthFunc(t.EQUAL);break;case Jl:t.depthFunc(t.GEQUAL);break;case $l:t.depthFunc(t.GREATER);break;case Kl:t.depthFunc(t.NOTEQUAL);break;default:t.depthFunc(t.LEQUAL)}ut=St}},setLocked:function(St){I=St},setClear:function(St){if(ft!==St){if(ft=St,Q)St=1-St;t.clearDepth(St)}},reset:function(){I=!1,Z=null,ut=null,ft=null,Q=!1}}}function s(){let I=!1,Q=null,Z=null,ut=null,ft=null,St=null,Bt=null,Ft=null,Et=null;return{setTest:function(Ot){if(!I)if(Ot)Dt(t.STENCIL_TEST);else ue(t.STENCIL_TEST)},setMask:function(Ot){if(Q!==Ot&&!I)t.stencilMask(Ot),Q=Ot},setFunc:function(Ot,Vt,At){if(Z!==Ot||ut!==Vt||ft!==At)t.stencilFunc(Ot,Vt,At),Z=Ot,ut=Vt,ft=At},setOp:function(Ot,Vt,At){if(St!==Ot||Bt!==Vt||Ft!==At)t.stencilOp(Ot,Vt,At),St=Ot,Bt=Vt,Ft=At},setLocked:function(Ot){I=Ot},setClear:function(Ot){if(Et!==Ot)t.clearStencil(Ot),Et=Ot},reset:function(){I=!1,Q=null,Z=null,ut=null,ft=null,St=null,Bt=null,Ft=null,Et=null}}}let r=new n,a=new i,o=new s,l={},c=new WeakMap,p={},h={},u={},d=new WeakMap,v=[],w=null,f=!1,m=null,M=null,T=null,S=null,E=null,P=null,g=null,y=new le(0,0,0),D=0,q=!1,N=null,X=null,rt=null,R=null,j=null,tt=t.getParameter(t.MAX_COMBINED_TEXTURE_IMAGE_UNITS),G=!1,pt=0,J=t.getParameter(t.VERSION);if(J.indexOf("WebGL")!==-1)pt=parseFloat(/^WebGL (\d)/.exec(J)[1]),G=pt>=1;else if(J.indexOf("OpenGL ES")!==-1)pt=parseFloat(/^OpenGL ES (\d)/.exec(J)[1]),G=pt>=2;let st=null,k={},It=t.getParameter(t.SCISSOR_BOX),nt=t.getParameter(t.VIEWPORT),ct=new Ae().fromArray(It),yt=new Ae().fromArray(nt);function Gt(I,Q,Z,ut){let ft=new Uint8Array(4),St=t.createTexture();t.bindTexture(I,St),t.texParameteri(I,t.TEXTURE_MIN_FILTER,t.NEAREST),t.texParameteri(I,t.TEXTURE_MAG_FILTER,t.NEAREST);for(let Bt=0;Bt<Z;Bt++)if(I===t.TEXTURE_3D||I===t.TEXTURE_2D_ARRAY)t.texImage3D(Q,0,t.RGBA,1,1,ut,0,t.RGBA,t.UNSIGNED_BYTE,ft);else t.texImage2D(Q+Bt,0,t.RGBA,1,1,0,t.RGBA,t.UNSIGNED_BYTE,ft);return St}let zt={};zt[t.TEXTURE_2D]=Gt(t.TEXTURE_2D,t.TEXTURE_2D,1),zt[t.TEXTURE_CUBE_MAP]=Gt(t.TEXTURE_CUBE_MAP,t.TEXTURE_CUBE_MAP_POSITIVE_X,6),zt[t.TEXTURE_2D_ARRAY]=Gt(t.TEXTURE_2D_ARRAY,t.TEXTURE_2D_ARRAY,1,1),zt[t.TEXTURE_3D]=Gt(t.TEXTURE_3D,t.TEXTURE_3D,1,1),r.setClear(0,0,0,1),a.setClear(1),o.setClear(0),Dt(t.DEPTH_TEST),a.setFunc(la),Le(!1),_e(ra),Dt(t.CULL_FACE),ge(ai);function Dt(I){if(p[I]!==!0)t.enable(I),p[I]=!0}function ue(I){if(p[I]!==!1)t.disable(I),p[I]=!1}function ee(I,Q){if(u[I]!==Q){if(t.bindFramebuffer(I,Q),u[I]=Q,I===t.DRAW_FRAMEBUFFER)u[t.FRAMEBUFFER]=Q;if(I===t.FRAMEBUFFER)u[t.DRAW_FRAMEBUFFER]=Q;return!0}return!1}function Jt(I,Q){let Z=v,ut=!1;if(I){if(Z=d.get(Q),Z===void 0)Z=[],d.set(Q,Z);let ft=I.textures;if(Z.length!==ft.length||Z[0]!==t.COLOR_ATTACHMENT0){for(let St=0,Bt=ft.length;St<Bt;St++)Z[St]=t.COLOR_ATTACHMENT0+St;Z.length=ft.length,ut=!0}}else if(Z[0]!==t.BACK)Z[0]=t.BACK,ut=!0;if(ut)t.drawBuffers(Z)}function ve(I){if(w!==I)return t.useProgram(I),w=I,!0;return!1}let ie={[zi]:t.FUNC_ADD,[Tl]:t.FUNC_SUBTRACT,[El]:t.FUNC_REVERSE_SUBTRACT};ie[Rl]=t.MIN,ie[Cl]=t.MAX;let Me={[Il]:t.ZERO,[Pl]:t.ONE,[Ll]:t.SRC_COLOR,[Dl]:t.SRC_ALPHA,[kl]:t.SRC_ALPHA_SATURATE,[Bl]:t.DST_COLOR,[Fl]:t.DST_ALPHA,[Nl]:t.ONE_MINUS_SRC_COLOR,[Ul]:t.ONE_MINUS_SRC_ALPHA,[zl]:t.ONE_MINUS_DST_COLOR,[Ol]:t.ONE_MINUS_DST_ALPHA,[Gl]:t.CONSTANT_COLOR,[Hl]:t.ONE_MINUS_CONSTANT_COLOR,[Wl]:t.CONSTANT_ALPHA,[Vl]:t.ONE_MINUS_CONSTANT_ALPHA};function ge(I,Q,Z,ut,ft,St,Bt,Ft,Et,Ot){if(I===ai){if(f===!0)ue(t.BLEND),f=!1;return}if(f===!1)Dt(t.BLEND),f=!0;if(I!==Al){if(I!==m||Ot!==q){if(M!==zi||E!==zi)t.blendEquation(t.FUNC_ADD),M=zi,E=zi;if(Ot)switch(I){case oi:t.blendFuncSeparate(t.ONE,t.ONE_MINUS_SRC_ALPHA,t.ONE,t.ONE_MINUS_SRC_ALPHA);break;case ss:t.blendFunc(t.ONE,t.ONE);break;case aa:t.blendFuncSeparate(t.ZERO,t.ONE_MINUS_SRC_COLOR,t.ZERO,t.ONE);break;case oa:t.blendFuncSeparate(t.DST_COLOR,t.ONE_MINUS_SRC_ALPHA,t.ZERO,t.ONE);break;default:Yt("WebGLState: Invalid blending: ",I);break}else switch(I){case oi:t.blendFuncSeparate(t.SRC_ALPHA,t.ONE_MINUS_SRC_ALPHA,t.ONE,t.ONE_MINUS_SRC_ALPHA);break;case ss:t.blendFuncSeparate(t.SRC_ALPHA,t.ONE,t.ONE,t.ONE);break;case aa:Yt("WebGLState: SubtractiveBlending requires material.premultipliedAlpha = true");break;case oa:Yt("WebGLState: MultiplyBlending requires material.premultipliedAlpha = true");break;default:Yt("WebGLState: Invalid blending: ",I);break}T=null,S=null,P=null,g=null,y.set(0,0,0),D=0,m=I,q=Ot}return}if(ft=ft||Q,St=St||Z,Bt=Bt||ut,Q!==M||ft!==E)t.blendEquationSeparate(ie[Q],ie[ft]),M=Q,E=ft;if(Z!==T||ut!==S||St!==P||Bt!==g)t.blendFuncSeparate(Me[Z],Me[ut],Me[St],Me[Bt]),T=Z,S=ut,P=St,g=Bt;if(Ft.equals(y)===!1||Et!==D)t.blendColor(Ft.r,Ft.g,Ft.b,Et),y.copy(Ft),D=Et;m=I,q=!1}function Re(I,Q){I.side===rn?ue(t.CULL_FACE):Dt(t.CULL_FACE);let Z=I.side===Qe;if(Q)Z=!Z;Le(Z),I.blending===oi&&I.transparent===!1?ge(ai):ge(I.blending,I.blendEquation,I.blendSrc,I.blendDst,I.blendEquationAlpha,I.blendSrcAlpha,I.blendDstAlpha,I.blendColor,I.blendAlpha,I.premultipliedAlpha),a.setFunc(I.depthFunc),a.setTest(I.depthTest),a.setMask(I.depthWrite),r.setMask(I.colorWrite);let ut=I.stencilWrite;if(o.setTest(ut),ut)o.setMask(I.stencilWriteMask),o.setFunc(I.stencilFunc,I.stencilRef,I.stencilFuncMask),o.setOp(I.stencilFail,I.stencilZFail,I.stencilZPass);Ue(I.polygonOffset,I.polygonOffsetFactor,I.polygonOffsetUnits),I.alphaToCoverage===!0?Dt(t.SAMPLE_ALPHA_TO_COVERAGE):ue(t.SAMPLE_ALPHA_TO_COVERAGE)}function Le(I){if(N!==I){if(I)t.frontFace(t.CW);else t.frontFace(t.CCW);N=I}}function _e(I){if(I!==Sl){if(Dt(t.CULL_FACE),I!==X)if(I===ra)t.cullFace(t.BACK);else if(I===bl)t.cullFace(t.FRONT);else t.cullFace(t.FRONT_AND_BACK)}else ue(t.CULL_FACE);X=I}function be(I){if(I!==rt){if(G)t.lineWidth(I);rt=I}}function Ue(I,Q,Z){if(I){if(Dt(t.POLYGON_OFFSET_FILL),R!==Q||j!==Z){if(R=Q,j=Z,a.getReversed())Q=-Q,Z=-Z;t.polygonOffset(Q,Z)}}else ue(t.POLYGON_OFFSET_FILL)}function Fe(I){if(I)Dt(t.SCISSOR_TEST);else ue(t.SCISSOR_TEST)}function Je(I){if(I===void 0)I=t.TEXTURE0+tt-1;if(st!==I)t.activeTexture(I),st=I}function U(I,Q,Z){if(Z===void 0)if(st===null)Z=t.TEXTURE0+tt-1;else Z=st;let ut=k[Z];if(ut===void 0)ut={type:void 0,texture:void 0},k[Z]=ut;if(ut.type!==I||ut.texture!==Q){if(st!==Z)t.activeTexture(Z),st=Z;t.bindTexture(I,Q||zt[I]),ut.type=I,ut.texture=Q}}function Ye(){let I=k[st];if(I!==void 0&&I.type!==void 0)t.bindTexture(I.type,null),I.type=void 0,I.texture=void 0}function de(){try{t.compressedTexImage2D(...arguments)}catch(I){Yt("WebGLState:",I)}}function xe(){try{t.compressedTexImage3D(...arguments)}catch(I){Yt("WebGLState:",I)}}function at(){try{t.texSubImage2D(...arguments)}catch(I){Yt("WebGLState:",I)}}function re(){try{t.texSubImage3D(...arguments)}catch(I){Yt("WebGLState:",I)}}function kt(){try{t.compressedTexSubImage2D(...arguments)}catch(I){Yt("WebGLState:",I)}}function Xt(){try{t.compressedTexSubImage3D(...arguments)}catch(I){Yt("WebGLState:",I)}}function Oe(){try{t.texStorage2D(...arguments)}catch(I){Yt("WebGLState:",I)}}function b(){try{t.texStorage3D(...arguments)}catch(I){Yt("WebGLState:",I)}}function _(){try{t.texImage2D(...arguments)}catch(I){Yt("WebGLState:",I)}}function B(){try{t.texImage3D(...arguments)}catch(I){Yt("WebGLState:",I)}}function ht(I){if(h[I]!==void 0)return h[I];else return t.getParameter(I)}function it(I,Q){if(h[I]!==Q)t.pixelStorei(I,Q),h[I]=Q}function mt(I){if(ct.equals(I)===!1)t.scissor(I.x,I.y,I.z,I.w),ct.copy(I)}function Mt(I){if(yt.equals(I)===!1)t.viewport(I.x,I.y,I.z,I.w),yt.copy(I)}function Y(I,Q){let Z=c.get(Q);if(Z===void 0)Z=new WeakMap,c.set(Q,Z);let ut=Z.get(I);if(ut===void 0)ut=t.getUniformBlockIndex(Q,I.name),t.uniformBlockBinding(Q,ut,ut),Z.set(I,ut)}function et(I,Q,Z){let ft=c.get(Q).get(I);if(l[ft]!==Z)t.bindBufferBase(t.UNIFORM_BUFFER,ft,Z),l[ft]=Z}function dt(){t.disable(t.BLEND),t.disable(t.CULL_FACE),t.disable(t.DEPTH_TEST),t.disable(t.POLYGON_OFFSET_FILL),t.disable(t.SCISSOR_TEST),t.disable(t.STENCIL_TEST),t.disable(t.SAMPLE_ALPHA_TO_COVERAGE),t.blendEquation(t.FUNC_ADD),t.blendFunc(t.ONE,t.ZERO),t.blendFuncSeparate(t.ONE,t.ZERO,t.ONE,t.ZERO),t.blendColor(0,0,0,0),t.colorMask(!0,!0,!0,!0),t.clearColor(0,0,0,0),t.depthMask(!0),t.depthFunc(t.LESS),a.setReversed(!1),t.clearDepth(1),t.stencilMask(4294967295),t.stencilFunc(t.ALWAYS,0,4294967295),t.stencilOp(t.KEEP,t.KEEP,t.KEEP),t.clearStencil(0),t.cullFace(t.BACK),t.frontFace(t.CCW),t.polygonOffset(0,0),t.activeTexture(t.TEXTURE0),t.bindFramebuffer(t.FRAMEBUFFER,null),t.bindFramebuffer(t.DRAW_FRAMEBUFFER,null),t.bindFramebuffer(t.READ_FRAMEBUFFER,null),t.useProgram(null),t.lineWidth(1),t.scissor(0,0,t.canvas.width,t.canvas.height),t.viewport(0,0,t.canvas.width,t.canvas.height),t.pixelStorei(t.PACK_ALIGNMENT,4),t.pixelStorei(t.UNPACK_ALIGNMENT,4),t.pixelStorei(t.UNPACK_FLIP_Y_WEBGL,!1),t.pixelStorei(t.UNPACK_PREMULTIPLY_ALPHA_WEBGL,!1),t.pixelStorei(t.UNPACK_COLORSPACE_CONVERSION_WEBGL,t.BROWSER_DEFAULT_WEBGL),t.pixelStorei(t.PACK_ROW_LENGTH,0),t.pixelStorei(t.PACK_SKIP_PIXELS,0),t.pixelStorei(t.PACK_SKIP_ROWS,0),t.pixelStorei(t.UNPACK_ROW_LENGTH,0),t.pixelStorei(t.UNPACK_IMAGE_HEIGHT,0),t.pixelStorei(t.UNPACK_SKIP_PIXELS,0),t.pixelStorei(t.UNPACK_SKIP_ROWS,0),t.pixelStorei(t.UNPACK_SKIP_IMAGES,0),p={},h={},st=null,k={},u={},l={},d=new WeakMap,v=[],w=null,f=!1,m=null,M=null,T=null,S=null,E=null,P=null,g=null,y=new le(0,0,0),D=0,q=!1,N=null,X=null,rt=null,R=null,j=null,ct.set(0,0,t.canvas.width,t.canvas.height),yt.set(0,0,t.canvas.width,t.canvas.height),r.reset(),a.reset(),o.reset()}return{buffers:{color:r,depth:a,stencil:o},enable:Dt,disable:ue,bindFramebuffer:ee,drawBuffers:Jt,useProgram:ve,setBlending:ge,setMaterial:Re,setFlipSided:Le,setCullFace:_e,setLineWidth:be,setPolygonOffset:Ue,setScissorTest:Fe,activeTexture:Je,bindTexture:U,unbindTexture:Ye,compressedTexImage2D:de,compressedTexImage3D:xe,texImage2D:_,texImage3D:B,pixelStorei:it,getParameter:ht,updateUBOMapping:Y,uniformBlockBinding:et,texStorage2D:Oe,texStorage3D:b,texSubImage2D:at,texSubImage3D:re,compressedTexSubImage2D:kt,compressedTexSubImage3D:Xt,scissor:mt,viewport:Mt,reset:dt}}function zm(t,e,n,i,s,r,a){let o=e.has("WEBGL_multisampled_render_to_texture")?e.get("WEBGL_multisampled_render_to_texture"):null,l=typeof navigator>"u"?!1:/OculusBrowser/g.test(navigator.userAgent),c=new fe,p=new WeakMap,h=new Set,u,d=new WeakMap,v=new Set,w=new FinalizationRegistry((b)=>v.delete(b)),f=!1;try{f=typeof OffscreenCanvas<"u"&&new OffscreenCanvas(1,1).getContext("2d")!==null}catch(b){}function m(b,_){return f?new OffscreenCanvas(b,_):ns("canvas")}function M(b,_,B){let ht=1,it=Oe(b);if(it.width>B||it.height>B)ht=B/Math.max(it.width,it.height);if(ht<1)if(typeof HTMLImageElement<"u"&&b instanceof HTMLImageElement||typeof HTMLCanvasElement<"u"&&b instanceof HTMLCanvasElement||typeof ImageBitmap<"u"&&b instanceof ImageBitmap||typeof VideoFrame<"u"&&b instanceof VideoFrame){let mt=Math.floor(ht*it.width),Mt=Math.floor(ht*it.height);if(u===void 0)u=m(mt,Mt);let Y=_?m(mt,Mt):u;return Y.width=mt,Y.height=Mt,Y.getContext("2d").drawImage(b,0,0,mt,Mt),Wt("WebGLRenderer: Texture has been resized from ("+it.width+"x"+it.height+") to ("+mt+"x"+Mt+")."),Y}else{if("data"in b)Wt("WebGLRenderer: Image in DataTexture is too big ("+it.width+"x"+it.height+").");return b}return b}function T(b){return b.generateMipmaps}function S(b){t.generateMipmap(b)}function E(b){if(b.isWebGLCubeRenderTarget)return t.TEXTURE_CUBE_MAP;if(b.isWebGL3DRenderTarget)return t.TEXTURE_3D;if(b.isWebGLArrayRenderTarget||b.isCompressedArrayTexture)return t.TEXTURE_2D_ARRAY;return t.TEXTURE_2D}function P(b,_,B,ht,it,mt=!1){if(b!==null){if(t[b]!==void 0)return t[b];Wt("WebGLRenderer: Attempt to use non-existing WebGL internal format '"+b+"'")}let Mt;if(ht){if(Mt=e.get("EXT_texture_norm16"),!Mt)Wt("WebGLRenderer: Unable to use normalized textures without EXT_texture_norm16 extension")}let Y=_;if(_===t.RED){if(B===t.FLOAT)Y=t.R32F;if(B===t.HALF_FLOAT)Y=t.R16F;if(B===t.UNSIGNED_BYTE)Y=t.R8;if(B===t.UNSIGNED_SHORT&&Mt)Y=Mt.R16_EXT;if(B===t.SHORT&&Mt)Y=Mt.R16_SNORM_EXT}if(_===t.RED_INTEGER){if(B===t.UNSIGNED_BYTE)Y=t.R8UI;if(B===t.UNSIGNED_SHORT)Y=t.R16UI;if(B===t.UNSIGNED_INT)Y=t.R32UI;if(B===t.BYTE)Y=t.R8I;if(B===t.SHORT)Y=t.R16I;if(B===t.INT)Y=t.R32I}if(_===t.RG){if(B===t.FLOAT)Y=t.RG32F;if(B===t.HALF_FLOAT)Y=t.RG16F;if(B===t.UNSIGNED_BYTE)Y=t.RG8;if(B===t.UNSIGNED_SHORT&&Mt)Y=Mt.RG16_EXT;if(B===t.SHORT&&Mt)Y=Mt.RG16_SNORM_EXT}if(_===t.RG_INTEGER){if(B===t.UNSIGNED_BYTE)Y=t.RG8UI;if(B===t.UNSIGNED_SHORT)Y=t.RG16UI;if(B===t.UNSIGNED_INT)Y=t.RG32UI;if(B===t.BYTE)Y=t.RG8I;if(B===t.SHORT)Y=t.RG16I;if(B===t.INT)Y=t.RG32I}if(_===t.RGB_INTEGER){if(B===t.UNSIGNED_BYTE)Y=t.RGB8UI;if(B===t.UNSIGNED_SHORT)Y=t.RGB16UI;if(B===t.UNSIGNED_INT)Y=t.RGB32UI;if(B===t.BYTE)Y=t.RGB8I;if(B===t.SHORT)Y=t.RGB16I;if(B===t.INT)Y=t.RGB32I}if(_===t.RGBA_INTEGER){if(B===t.UNSIGNED_BYTE)Y=t.RGBA8UI;if(B===t.UNSIGNED_SHORT)Y=t.RGBA16UI;if(B===t.UNSIGNED_INT)Y=t.RGBA32UI;if(B===t.BYTE)Y=t.RGBA8I;if(B===t.SHORT)Y=t.RGBA16I;if(B===t.INT)Y=t.RGBA32I}if(_===t.RGB){if(B===t.UNSIGNED_SHORT&&Mt)Y=Mt.RGB16_EXT;if(B===t.SHORT&&Mt)Y=Mt.RGB16_SNORM_EXT;if(B===t.UNSIGNED_INT_5_9_9_9_REV)Y=t.RGB9_E5;if(B===t.UNSIGNED_INT_10F_11F_11F_REV)Y=t.R11F_G11F_B10F}if(_===t.RGBA){let et=mt?to:ae.getTransfer(it);if(B===t.FLOAT)Y=t.RGBA32F;if(B===t.HALF_FLOAT)Y=t.RGBA16F;if(B===t.UNSIGNED_BYTE)Y=et===ye?t.SRGB8_ALPHA8:t.RGBA8;if(B===t.UNSIGNED_SHORT&&Mt)Y=Mt.RGBA16_EXT;if(B===t.SHORT&&Mt)Y=Mt.RGBA16_SNORM_EXT;if(B===t.UNSIGNED_SHORT_4_4_4_4)Y=t.RGBA4;if(B===t.UNSIGNED_SHORT_5_5_5_1)Y=t.RGB5_A1}if(Y===t.R16F||Y===t.R32F||Y===t.RG16F||Y===t.RG32F||Y===t.RGBA16F||Y===t.RGBA32F)e.get("EXT_color_buffer_float");return Y}function g(b,_){let B;if(b){if(_===null||_===Zn||_===li)B=t.DEPTH24_STENCIL8;else if(_===Un)B=t.DEPTH32F_STENCIL8;else if(_===os)B=t.DEPTH24_STENCIL8,Wt("DepthTexture: 16 bit depth attachment is not supported with stencil. Using 24-bit attachment.")}else if(_===null||_===Zn||_===li)B=t.DEPTH_COMPONENT24;else if(_===Un)B=t.DEPTH_COMPONENT32F;else if(_===os)B=t.DEPTH_COMPONENT16;return B}function y(b,_){if(T(b)===!0||b.isFramebufferTexture&&b.minFilter!==qn&&b.minFilter!==ln)return Math.log2(Math.max(_.width,_.height))+1;else if(b.mipmaps.length>0)return b.mipmaps.length;else if(b.isCompressedTexture&&Array.isArray(b.image))return _.mipmaps.length;else return 1}function D(b){let _=new WeakRef(b);i.get(b).__ref=_,v.add(_),w.register(b,_,_)}function q(b){let _=i.get(b).__ref;if(_===void 0)return;v.delete(_),w.unregister(_)}function N(b){rt(b.target)}function X(b){R(b.target)}function rt(b){if(b.removeEventListener("dispose",N),q(b),tt(b),b.isVideoTexture)p.delete(b);if(b.isHTMLTexture)h.delete(b)}function R(b){b.removeEventListener("dispose",X),q(b),pt(b)}function j(){for(let b of v){let _=b.deref();if(_===void 0)continue;if(_.isRenderTarget===!0)R(_);else rt(_)}v.clear(),h.clear()}function tt(b){let _=i.get(b);if(_.__webglInit===void 0)return;let B=b.source,ht=d.get(B);if(ht){let it=ht[_.__cacheKey];if(it.usedTimes--,it.usedTimes===0)G(b);if(Object.keys(ht).length===0)d.delete(B)}i.remove(b)}function G(b){let _=i.get(b);t.deleteTexture(_.__webglTexture);let B=b.source,ht=d.get(B);delete ht[_.__cacheKey],a.memory.textures--}function pt(b){let _=i.get(b);if(_.__depthDisposeCallback)_.__depthDisposeCallback();let B=b.depthTexture;if(B&&B.renderTarget===b)rt(B);if(b.isWebGLCubeRenderTarget)for(let it=0;it<6;it++){if(Array.isArray(_.__webglFramebuffer[it]))for(let mt=0;mt<_.__webglFramebuffer[it].length;mt++)t.deleteFramebuffer(_.__webglFramebuffer[it][mt]);else t.deleteFramebuffer(_.__webglFramebuffer[it]);if(_.__webglDepthbuffer)t.deleteRenderbuffer(_.__webglDepthbuffer[it])}else{if(Array.isArray(_.__webglFramebuffer))for(let it=0;it<_.__webglFramebuffer.length;it++)t.deleteFramebuffer(_.__webglFramebuffer[it]);else t.deleteFramebuffer(_.__webglFramebuffer);if(_.__webglDepthbuffer)t.deleteRenderbuffer(_.__webglDepthbuffer);if(_.__webglMultisampledFramebuffer)t.deleteFramebuffer(_.__webglMultisampledFramebuffer);if(_.__webglColorRenderbuffer){for(let it=0;it<_.__webglColorRenderbuffer.length;it++)if(_.__webglColorRenderbuffer[it])t.deleteRenderbuffer(_.__webglColorRenderbuffer[it])}if(_.__webglDepthRenderbuffer)t.deleteRenderbuffer(_.__webglDepthRenderbuffer)}let ht=b.textures;for(let it=0,mt=ht.length;it<mt;it++){let Mt=i.get(ht[it]);if(Mt.__webglTexture)t.deleteTexture(Mt.__webglTexture),a.memory.textures--;i.remove(ht[it])}i.remove(b)}let J=0;function st(){J=0}function k(){return J}function It(b){J=b}function nt(){let b=J;if(b>=s.maxTextures)Wt("WebGLTextures: Trying to use "+(b+1)+" texture units while this GPU supports only "+s.maxTextures);return J+=1,b}function ct(b){let _=[];return _.push(b.wrapS),_.push(b.wrapT),_.push(b.wrapR||0),_.push(b.magFilter),_.push(b.minFilter),_.push(b.anisotropy),_.push(b.internalFormat),_.push(b.format),_.push(b.type),_.push(b.generateMipmaps),_.push(b.mipmapsAutoUpdate),_.push(b.premultiplyAlpha),_.push(b.flipY),_.push(b.unpackAlignment),_.push(b.colorSpace),_.join()}function yt(b,_){let B=i.get(b);if(b.isVideoTexture)kt(b);if(b.isRenderTargetTexture===!1&&b.isExternalTexture!==!0&&b.version>0&&B.__version!==b.version){let ht=b.image;if(ht===null)Wt("WebGLRenderer: Texture marked for update but no image data found.");else if(ht.complete===!1)Wt("WebGLRenderer: Texture marked for update but image is incomplete");else{Re(B,b,_);return}}else if(b.isExternalTexture)B.__webglTexture=b.sourceTexture?b.sourceTexture:null;n.bindTexture(t.TEXTURE_2D,B.__webglTexture,t.TEXTURE0+_)}function Gt(b,_){let B=i.get(b);if(b.isRenderTargetTexture===!1&&b.version>0&&B.__version!==b.version){Re(B,b,_);return}else if(b.isExternalTexture)B.__webglTexture=b.sourceTexture?b.sourceTexture:null;n.bindTexture(t.TEXTURE_2D_ARRAY,B.__webglTexture,t.TEXTURE0+_)}function zt(b,_){let B=i.get(b);if(b.isRenderTargetTexture===!1&&b.version>0&&B.__version!==b.version){Re(B,b,_);return}n.bindTexture(t.TEXTURE_3D,B.__webglTexture,t.TEXTURE0+_)}function Dt(b,_){let B=i.get(b);if(b.version>0&&B.__version!==b.version){Le(B,b,_);return}n.bindTexture(t.TEXTURE_CUBE_MAP,B.__webglTexture,t.TEXTURE0+_)}let ue={[ec]:t.REPEAT,[er]:t.CLAMP_TO_EDGE,[nc]:t.MIRRORED_REPEAT},ee={[qn]:t.NEAREST,[ic]:t.NEAREST_MIPMAP_NEAREST,[as]:t.NEAREST_MIPMAP_LINEAR,[ln]:t.LINEAR,[nr]:t.LINEAR_MIPMAP_NEAREST,[Yn]:t.LINEAR_MIPMAP_LINEAR},Jt={[fc]:t.NEVER,[io]:t.ALWAYS,[pc]:t.LESS,[eo]:t.LEQUAL,[mc]:t.EQUAL,[no]:t.GEQUAL,[gc]:t.GREATER,[_c]:t.NOTEQUAL};function ve(b,_){if(_.type===Un&&e.has("OES_texture_float_linear")===!1&&(_.magFilter===ln||_.magFilter===nr||_.magFilter===as||_.magFilter===Yn||_.minFilter===ln||_.minFilter===nr||_.minFilter===as||_.minFilter===Yn))Wt("WebGLRenderer: Unable to use linear filtering with floating point textures. OES_texture_float_linear not supported on this device.");if(t.texParameteri(b,t.TEXTURE_WRAP_S,ue[_.wrapS]),t.texParameteri(b,t.TEXTURE_WRAP_T,ue[_.wrapT]),b===t.TEXTURE_3D||b===t.TEXTURE_2D_ARRAY)t.texParameteri(b,t.TEXTURE_WRAP_R,ue[_.wrapR]);if(t.texParameteri(b,t.TEXTURE_MAG_FILTER,ee[_.magFilter]),t.texParameteri(b,t.TEXTURE_MIN_FILTER,ee[_.minFilter]),_.compareFunction)t.texParameteri(b,t.TEXTURE_COMPARE_MODE,t.COMPARE_REF_TO_TEXTURE),t.texParameteri(b,t.TEXTURE_COMPARE_FUNC,Jt[_.compareFunction]);if(e.has("EXT_texture_filter_anisotropic")===!0){if(_.magFilter===qn)return;if(_.minFilter!==as&&_.minFilter!==Yn)return;if(_.type===Un&&e.has("OES_texture_float_linear")===!1)return;if(_.anisotropy>1||i.get(_).__currentAnisotropy){let B=e.get("EXT_texture_filter_anisotropic");t.texParameterf(b,B.TEXTURE_MAX_ANISOTROPY_EXT,Math.min(_.anisotropy,s.getMaxAnisotropy())),i.get(_).__currentAnisotropy=_.anisotropy}}}function ie(b,_){let B=!1;if(b.__webglInit===void 0)b.__webglInit=!0,_.addEventListener("dispose",N),D(_);let ht=_.source,it=d.get(ht);if(it===void 0)it={},d.set(ht,it);let mt=ct(_);if(mt!==b.__cacheKey){if(it[mt]===void 0)it[mt]={texture:t.createTexture(),usedTimes:0},a.memory.textures++,B=!0;it[mt].usedTimes++;let Mt=it[b.__cacheKey];if(Mt!==void 0){if(it[b.__cacheKey].usedTimes--,Mt.usedTimes===0)G(_)}b.__cacheKey=mt,b.__webglTexture=it[mt].texture}return B}function Me(b,_,B){return Math.floor(Math.floor(b/B)/_)}function ge(b,_,B,ht){let mt=b.updateRanges;if(mt.length===0)n.texSubImage2D(t.TEXTURE_2D,0,0,0,_.width,_.height,B,ht,_.data);else{mt.sort((I,Q)=>I.start-Q.start);let Mt=0;for(let I=1;I<mt.length;I++){let Q=mt[Mt],Z=mt[I],ut=Q.start+Q.count,ft=Me(Z.start,_.width,4),St=Me(Q.start,_.width,4);if(Z.start<=ut+1&&ft===St&&Me(Z.start+Z.count-1,_.width,4)===ft)Q.count=Math.max(Q.count,Z.start+Z.count-Q.start);else++Mt,mt[Mt]=Z}mt.length=Mt+1;let Y=n.getParameter(t.UNPACK_ROW_LENGTH),et=n.getParameter(t.UNPACK_SKIP_PIXELS),dt=n.getParameter(t.UNPACK_SKIP_ROWS);n.pixelStorei(t.UNPACK_ROW_LENGTH,_.width);for(let I=0,Q=mt.length;I<Q;I++){let Z=mt[I],ut=Math.floor(Z.start/4),ft=Math.ceil(Z.count/4),St=ut%_.width,Bt=Math.floor(ut/_.width),Ft=ft,Et=1;n.pixelStorei(t.UNPACK_SKIP_PIXELS,St),n.pixelStorei(t.UNPACK_SKIP_ROWS,Bt),n.texSubImage2D(t.TEXTURE_2D,0,St,Bt,Ft,1,B,ht,_.data)}b.clearUpdateRanges(),n.pixelStorei(t.UNPACK_ROW_LENGTH,Y),n.pixelStorei(t.UNPACK_SKIP_PIXELS,et),n.pixelStorei(t.UNPACK_SKIP_ROWS,dt)}}function Re(b,_,B){let ht=t.TEXTURE_2D;if(_.isDataArrayTexture||_.isCompressedArrayTexture)ht=t.TEXTURE_2D_ARRAY;if(_.isData3DTexture)ht=t.TEXTURE_3D;let it=ie(b,_),mt=_.source;n.bindTexture(ht,b.__webglTexture,t.TEXTURE0+B);let Mt=i.get(mt);if(mt.version!==Mt.__version||it===!0){if(n.activeTexture(t.TEXTURE0+B),(typeof ImageBitmap<"u"&&_.image instanceof ImageBitmap)===!1){let Et=ae.getPrimaries(ae.workingColorSpace),Ot=_.colorSpace===hi?null:ae.getPrimaries(_.colorSpace),Vt=_.colorSpace===hi||Et===Ot?t.NONE:t.BROWSER_DEFAULT_WEBGL;n.pixelStorei(t.UNPACK_FLIP_Y_WEBGL,_.flipY),n.pixelStorei(t.UNPACK_PREMULTIPLY_ALPHA_WEBGL,_.premultiplyAlpha),n.pixelStorei(t.UNPACK_COLORSPACE_CONVERSION_WEBGL,Vt)}n.pixelStorei(t.UNPACK_ALIGNMENT,_.unpackAlignment);let et=M(_.image,!1,s.maxTextureSize);et=Xt(_,et);let dt=r.convert(_.format,_.colorSpace),I=r.convert(_.type),Q=P(_.internalFormat,dt,I,_.normalized,_.colorSpace,_.isVideoTexture);ve(ht,_);let Z,ut=_.mipmaps,ft=_.isVideoTexture!==!0,St=Mt.__version===void 0||it===!0,Bt=mt.dataReady,Ft=y(_,et);if(_.isDepthTexture){if(Q=g(_.format===Jn,_.type),St)if(ft)n.texStorage2D(t.TEXTURE_2D,1,Q,et.width,et.height);else n.texImage2D(t.TEXTURE_2D,0,Q,et.width,et.height,0,dt,I,null)}else if(_.isDataTexture)if(ut.length>0){if(ft&&St)n.texStorage2D(t.TEXTURE_2D,Ft,Q,ut[0].width,ut[0].height);for(let Et=0,Ot=ut.length;Et<Ot;Et++)if(Z=ut[Et],ft){if(Bt)n.texSubImage2D(t.TEXTURE_2D,Et,0,0,Z.width,Z.height,dt,I,Z.data)}else n.texImage2D(t.TEXTURE_2D,Et,Q,Z.width,Z.height,0,dt,I,Z.data);_.generateMipmaps=!1}else if(ft){if(St)n.texStorage2D(t.TEXTURE_2D,Ft,Q,et.width,et.height);if(Bt)ge(_,et,dt,I)}else n.texImage2D(t.TEXTURE_2D,0,Q,et.width,et.height,0,dt,I,et.data);else if(_.isCompressedTexture)if(_.isCompressedArrayTexture){if(ft&&St)n.texStorage3D(t.TEXTURE_2D_ARRAY,Ft,Q,ut[0].width,ut[0].height,et.depth);for(let Et=0,Ot=ut.length;Et<Ot;Et++)if(Z=ut[Et],_.format!==Fn)if(dt!==null)if(ft){if(Bt)if(_.layerUpdates.size>0){let Vt=Uo(Z.width,Z.height,_.format,_.type);for(let At of _.layerUpdates){let ce=Z.data.subarray(At*Vt/Z.data.BYTES_PER_ELEMENT,(At+1)*Vt/Z.data.BYTES_PER_ELEMENT);n.compressedTexSubImage3D(t.TEXTURE_2D_ARRAY,Et,0,0,At,Z.width,Z.height,1,dt,ce)}}else n.compressedTexSubImage3D(t.TEXTURE_2D_ARRAY,Et,0,0,0,Z.width,Z.height,et.depth,dt,Z.data)}else n.compressedTexImage3D(t.TEXTURE_2D_ARRAY,Et,Q,Z.width,Z.height,et.depth,0,Z.data,0,0);else Wt("WebGLRenderer: Attempt to load unsupported compressed texture format in .uploadTexture()");else if(ft){if(Bt)n.texSubImage3D(t.TEXTURE_2D_ARRAY,Et,0,0,0,Z.width,Z.height,et.depth,dt,I,Z.data)}else n.texImage3D(t.TEXTURE_2D_ARRAY,Et,Q,Z.width,Z.height,et.depth,0,dt,I,Z.data);if(_.layerUpdates.size>0)_.clearLayerUpdates()}else{if(ft&&St)n.texStorage2D(t.TEXTURE_2D,Ft,Q,ut[0].width,ut[0].height);for(let Et=0,Ot=ut.length;Et<Ot;Et++)if(Z=ut[Et],_.format!==Fn)if(dt!==null)if(ft){if(Bt)n.compressedTexSubImage2D(t.TEXTURE_2D,Et,0,0,Z.width,Z.height,dt,Z.data)}else n.compressedTexImage2D(t.TEXTURE_2D,Et,Q,Z.width,Z.height,0,Z.data);else Wt("WebGLRenderer: Attempt to load unsupported compressed texture format in .uploadTexture()");else if(ft){if(Bt)n.texSubImage2D(t.TEXTURE_2D,Et,0,0,Z.width,Z.height,dt,I,Z.data)}else n.texImage2D(t.TEXTURE_2D,Et,Q,Z.width,Z.height,0,dt,I,Z.data)}else if(_.isDataArrayTexture)if(ft){if(St)n.texStorage3D(t.TEXTURE_2D_ARRAY,Ft,Q,et.width,et.height,et.depth);if(Bt)if(_.layerUpdates.size>0){let Et=Uo(et.width,et.height,_.format,_.type);for(let Ot of _.layerUpdates){let Vt=et.data.subarray(Ot*Et/et.data.BYTES_PER_ELEMENT,(Ot+1)*Et/et.data.BYTES_PER_ELEMENT);n.texSubImage3D(t.TEXTURE_2D_ARRAY,0,0,0,Ot,et.width,et.height,1,dt,I,Vt)}_.clearLayerUpdates()}else n.texSubImage3D(t.TEXTURE_2D_ARRAY,0,0,0,0,et.width,et.height,et.depth,dt,I,et.data)}else n.texImage3D(t.TEXTURE_2D_ARRAY,0,Q,et.width,et.height,et.depth,0,dt,I,et.data);else if(_.isData3DTexture)if(ft){if(St)n.texStorage3D(t.TEXTURE_3D,Ft,Q,et.width,et.height,et.depth);if(Bt)n.texSubImage3D(t.TEXTURE_3D,0,0,0,0,et.width,et.height,et.depth,dt,I,et.data)}else n.texImage3D(t.TEXTURE_3D,0,Q,et.width,et.height,et.depth,0,dt,I,et.data);else if(_.isFramebufferTexture){if(St)if(ft)n.texStorage2D(t.TEXTURE_2D,Ft,Q,et.width,et.height);else{let Et=et.width,Ot=et.height;for(let Vt=0;Vt<Ft;Vt++)n.texImage2D(t.TEXTURE_2D,Vt,Q,Et,Ot,0,dt,I,null),Et>>=1,Ot>>=1}}else if(_.isHTMLTexture){if("texElementImage2D"in t){let Et=t.canvas;if(!Et.hasAttribute("layoutsubtree"))Et.setAttribute("layoutsubtree","true");if(et.parentNode!==Et){Et.appendChild(et),h.add(_),Et.onpaint=(Ot)=>{let Vt=Ot.changedElements;for(let At of h)if(Vt.includes(At.image))At.needsUpdate=!0},Et.requestPaint();return}if(t.texElementImage2D.length===3)t.texElementImage2D(t.TEXTURE_2D,t.RGBA8,et);else{let{RGBA:Vt,RGBA:At,UNSIGNED_BYTE:ce}=t;t.texElementImage2D(t.TEXTURE_2D,0,Vt,At,ce,et)}t.texParameteri(t.TEXTURE_2D,t.TEXTURE_MIN_FILTER,t.LINEAR),t.texParameteri(t.TEXTURE_2D,t.TEXTURE_WRAP_S,t.CLAMP_TO_EDGE),t.texParameteri(t.TEXTURE_2D,t.TEXTURE_WRAP_T,t.CLAMP_TO_EDGE)}}else if(ut.length>0){if(ft&&St){let Et=Oe(ut[0]);n.texStorage2D(t.TEXTURE_2D,Ft,Q,Et.width,Et.height)}for(let Et=0,Ot=ut.length;Et<Ot;Et++)if(Z=ut[Et],ft){if(Bt)n.texSubImage2D(t.TEXTURE_2D,Et,0,0,dt,I,Z)}else n.texImage2D(t.TEXTURE_2D,Et,Q,dt,I,Z);_.generateMipmaps=!1}else if(ft){if(St){let Et=Oe(et);n.texStorage2D(t.TEXTURE_2D,Ft,Q,Et.width,Et.height)}if(Bt)n.texSubImage2D(t.TEXTURE_2D,0,0,0,dt,I,et)}else n.texImage2D(t.TEXTURE_2D,0,Q,dt,I,et);if(T(_)&&_.mipmapsAutoUpdate===!0)S(ht);if(Mt.__version=mt.version,_.onUpdate)_.onUpdate(_)}b.__version=_.version}function Le(b,_,B){if(_.image.length!==6)return;let ht=ie(b,_),it=_.source;n.bindTexture(t.TEXTURE_CUBE_MAP,b.__webglTexture,t.TEXTURE0+B);let mt=i.get(it);if(it.version!==mt.__version||ht===!0){n.activeTexture(t.TEXTURE0+B);let Mt=ae.getPrimaries(ae.workingColorSpace),Y=_.colorSpace===hi?null:ae.getPrimaries(_.colorSpace),et=_.colorSpace===hi||Mt===Y?t.NONE:t.BROWSER_DEFAULT_WEBGL;n.pixelStorei(t.UNPACK_FLIP_Y_WEBGL,_.flipY),n.pixelStorei(t.UNPACK_PREMULTIPLY_ALPHA_WEBGL,_.premultiplyAlpha),n.pixelStorei(t.UNPACK_ALIGNMENT,_.unpackAlignment),n.pixelStorei(t.UNPACK_COLORSPACE_CONVERSION_WEBGL,et);let dt=_.isCompressedTexture||_.image[0].isCompressedTexture,I=_.image[0]&&_.image[0].isDataTexture,Q=[];for(let At=0;At<6;At++){if(!dt&&!I)Q[At]=M(_.image[At],!0,s.maxCubemapSize);else Q[At]=I?_.image[At].image:_.image[At];Q[At]=Xt(_,Q[At])}let Z=Q[0],ut=r.convert(_.format,_.colorSpace),ft=r.convert(_.type),St=P(_.internalFormat,ut,ft,_.normalized,_.colorSpace),Bt=_.isVideoTexture!==!0,Ft=mt.__version===void 0||ht===!0,Et=it.dataReady,Ot=y(_,Z);ve(t.TEXTURE_CUBE_MAP,_);let Vt;if(_.isDepthTexture){if(St=g(_.format===Jn,_.type),Ft)if(Bt)n.texStorage2D(t.TEXTURE_CUBE_MAP,1,St,Z.width,Z.height);else for(let At=0;At<6;At++)n.texImage2D(t.TEXTURE_CUBE_MAP_POSITIVE_X+At,0,St,Z.width,Z.height,0,ut,ft,null)}else if(dt){if(Bt&&Ft)n.texStorage2D(t.TEXTURE_CUBE_MAP,Ot,St,Z.width,Z.height);for(let At=0;At<6;At++){Vt=Q[At].mipmaps;for(let ce=0;ce<Vt.length;ce++){let Ce=Vt[ce];if(_.format!==Fn)if(ut!==null)if(Bt){if(Et)n.compressedTexSubImage2D(t.TEXTURE_CUBE_MAP_POSITIVE_X+At,ce,0,0,Ce.width,Ce.height,ut,Ce.data)}else n.compressedTexImage2D(t.TEXTURE_CUBE_MAP_POSITIVE_X+At,ce,St,Ce.width,Ce.height,0,Ce.data);else Wt("WebGLRenderer: Attempt to load unsupported compressed texture format in .setTextureCube()");else if(Bt){if(Et)n.texSubImage2D(t.TEXTURE_CUBE_MAP_POSITIVE_X+At,ce,0,0,Ce.width,Ce.height,ut,ft,Ce.data)}else n.texImage2D(t.TEXTURE_CUBE_MAP_POSITIVE_X+At,ce,St,Ce.width,Ce.height,0,ut,ft,Ce.data)}}}else{if(Vt=_.mipmaps,Bt&&Ft){if(Vt.length>0)Ot++;let At=Oe(Q[0]);n.texStorage2D(t.TEXTURE_CUBE_MAP,Ot,St,At.width,At.height)}for(let At=0;At<6;At++)if(I){if(Bt){if(Et)n.texSubImage2D(t.TEXTURE_CUBE_MAP_POSITIVE_X+At,0,0,0,Q[At].width,Q[At].height,ut,ft,Q[At].data)}else n.texImage2D(t.TEXTURE_CUBE_MAP_POSITIVE_X+At,0,St,Q[At].width,Q[At].height,0,ut,ft,Q[At].data);for(let ce=0;ce<Vt.length;ce++){let dn=Vt[ce].image[At].image;if(Bt){if(Et)n.texSubImage2D(t.TEXTURE_CUBE_MAP_POSITIVE_X+At,ce+1,0,0,dn.width,dn.height,ut,ft,dn.data)}else n.texImage2D(t.TEXTURE_CUBE_MAP_POSITIVE_X+At,ce+1,St,dn.width,dn.height,0,ut,ft,dn.data)}}else{if(Bt){if(Et)n.texSubImage2D(t.TEXTURE_CUBE_MAP_POSITIVE_X+At,0,0,0,ut,ft,Q[At])}else n.texImage2D(t.TEXTURE_CUBE_MAP_POSITIVE_X+At,0,St,ut,ft,Q[At]);for(let ce=0;ce<Vt.length;ce++){let Ce=Vt[ce];if(Bt){if(Et)n.texSubImage2D(t.TEXTURE_CUBE_MAP_POSITIVE_X+At,ce+1,0,0,ut,ft,Ce.image[At])}else n.texImage2D(t.TEXTURE_CUBE_MAP_POSITIVE_X+At,ce+1,St,ut,ft,Ce.image[At])}}}if(T(_)&&_.mipmapsAutoUpdate===!0)S(t.TEXTURE_CUBE_MAP);if(mt.__version=it.version,_.onUpdate)_.onUpdate(_)}b.__version=_.version}function _e(b,_,B,ht,it,mt){let Mt=r.convert(B.format,B.colorSpace),Y=r.convert(B.type),et=P(B.internalFormat,Mt,Y,B.normalized,B.colorSpace),dt=i.get(_),I=i.get(B);if(I.__renderTarget=_,!dt.__hasExternalTextures){let Q=Math.max(1,_.width>>mt),Z=Math.max(1,_.height>>mt);if(it===t.TEXTURE_3D||it===t.TEXTURE_2D_ARRAY)n.texImage3D(it,mt,et,Q,Z,_.depth,0,Mt,Y,null);else n.texImage2D(it,mt,et,Q,Z,0,Mt,Y,null)}if(n.bindFramebuffer(t.FRAMEBUFFER,b),re(_))o.framebufferTexture2DMultisampleEXT(t.FRAMEBUFFER,ht,it,I.__webglTexture,0,at(_));else if(it===t.TEXTURE_2D||it>=t.TEXTURE_CUBE_MAP_POSITIVE_X&&it<=t.TEXTURE_CUBE_MAP_NEGATIVE_Z)t.framebufferTexture2D(t.FRAMEBUFFER,ht,it,I.__webglTexture,mt);n.bindFramebuffer(t.FRAMEBUFFER,null)}function be(b,_,B){if(t.bindRenderbuffer(t.RENDERBUFFER,b),_.depthBuffer){let ht=_.depthTexture,it=ht&&ht.isDepthTexture?ht.type:null,mt=g(_.stencilBuffer,it),Mt=_.stencilBuffer?t.DEPTH_STENCIL_ATTACHMENT:t.DEPTH_ATTACHMENT;if(re(_))o.renderbufferStorageMultisampleEXT(t.RENDERBUFFER,at(_),mt,_.width,_.height);else if(B)t.renderbufferStorageMultisample(t.RENDERBUFFER,at(_),mt,_.width,_.height);else t.renderbufferStorage(t.RENDERBUFFER,mt,_.width,_.height);t.framebufferRenderbuffer(t.FRAMEBUFFER,Mt,t.RENDERBUFFER,b)}else{let ht=_.textures;for(let it=0;it<ht.length;it++){let mt=ht[it],Mt=r.convert(mt.format,mt.colorSpace),Y=r.convert(mt.type),et=P(mt.internalFormat,Mt,Y,mt.normalized,mt.colorSpace);if(re(_))o.renderbufferStorageMultisampleEXT(t.RENDERBUFFER,at(_),et,_.width,_.height);else if(B)t.renderbufferStorageMultisample(t.RENDERBUFFER,at(_),et,_.width,_.height);else t.renderbufferStorage(t.RENDERBUFFER,et,_.width,_.height)}}t.bindRenderbuffer(t.RENDERBUFFER,null)}function Ue(b,_,B){let ht=_.isWebGLCubeRenderTarget===!0;if(n.bindFramebuffer(t.FRAMEBUFFER,b),!(_.depthTexture&&_.depthTexture.isDepthTexture))throw Error("THREE.WebGLTextures: renderTarget.depthTexture must be an instance of THREE.DepthTexture.");let it=i.get(_.depthTexture);it.__renderTarget=_;let mt=ht?_.depthTexture.image[0]:_.depthTexture.image;if(!it.__webglTexture||mt.width!==_.width||mt.height!==_.height)mt.width=_.width,mt.height=_.height,_.depthTexture.needsUpdate=!0;if(ht)Dt(_.depthTexture,0);else yt(_.depthTexture,0);let Mt=it.__webglTexture,Y=at(_),et=ht?t.TEXTURE_CUBE_MAP_POSITIVE_X+B:t.TEXTURE_2D,dt=_.depthTexture.format===Jn?t.DEPTH_STENCIL_ATTACHMENT:t.DEPTH_ATTACHMENT;if(_.depthTexture.format===Gi)if(re(_))o.framebufferTexture2DMultisampleEXT(t.FRAMEBUFFER,dt,et,Mt,0,Y);else t.framebufferTexture2D(t.FRAMEBUFFER,dt,et,Mt,0);else if(_.depthTexture.format===Jn)if(re(_))o.framebufferTexture2DMultisampleEXT(t.FRAMEBUFFER,dt,et,Mt,0,Y);else t.framebufferTexture2D(t.FRAMEBUFFER,dt,et,Mt,0);else throw Error("THREE.WebGLTextures: Unknown depthTexture format.")}function Fe(b){let _=i.get(b),B=b.isWebGLCubeRenderTarget===!0;if(_.__boundDepthTexture!==b.depthTexture){let ht=b.depthTexture;if(_.__depthDisposeCallback)_.__depthDisposeCallback();if(ht){let it=()=>{delete _.__boundDepthTexture,delete _.__depthDisposeCallback,ht.removeEventListener("dispose",it)};ht.addEventListener("dispose",it),_.__depthDisposeCallback=it}_.__boundDepthTexture=ht}if(b.depthTexture&&!_.__autoAllocateDepthBuffer)if(B)for(let ht=0;ht<6;ht++)Ue(_.__webglFramebuffer[ht],b,ht);else if(b.texture.mipmaps.length>0)Ue(_.__webglFramebuffer[0],b,0);else Ue(_.__webglFramebuffer,b,0);else if(B){_.__webglDepthbuffer=[];for(let ht=0;ht<6;ht++)if(n.bindFramebuffer(t.FRAMEBUFFER,_.__webglFramebuffer[ht]),_.__webglDepthbuffer[ht]===void 0)_.__webglDepthbuffer[ht]=t.createRenderbuffer(),be(_.__webglDepthbuffer[ht],b,!1);else{let it=b.stencilBuffer?t.DEPTH_STENCIL_ATTACHMENT:t.DEPTH_ATTACHMENT,mt=_.__webglDepthbuffer[ht];t.bindRenderbuffer(t.RENDERBUFFER,mt),t.framebufferRenderbuffer(t.FRAMEBUFFER,it,t.RENDERBUFFER,mt)}}else{if(b.texture.mipmaps.length>0)n.bindFramebuffer(t.FRAMEBUFFER,_.__webglFramebuffer[0]);else n.bindFramebuffer(t.FRAMEBUFFER,_.__webglFramebuffer);if(_.__webglDepthbuffer===void 0)_.__webglDepthbuffer=t.createRenderbuffer(),be(_.__webglDepthbuffer,b,!1);else{let it=b.stencilBuffer?t.DEPTH_STENCIL_ATTACHMENT:t.DEPTH_ATTACHMENT,mt=_.__webglDepthbuffer;t.bindRenderbuffer(t.RENDERBUFFER,mt),t.framebufferRenderbuffer(t.FRAMEBUFFER,it,t.RENDERBUFFER,mt)}}n.bindFramebuffer(t.FRAMEBUFFER,null)}function Je(b,_,B){let ht=i.get(b);if(_!==void 0)_e(ht.__webglFramebuffer,b,b.texture,t.COLOR_ATTACHMENT0,t.TEXTURE_2D,0);if(B!==void 0)Fe(b)}function U(b){let _=b.texture,B=i.get(b),ht=i.get(_);b.addEventListener("dispose",X),D(b);let it=b.textures,mt=b.isWebGLCubeRenderTarget===!0,Mt=it.length>1;if(!Mt){if(ht.__webglTexture===void 0)ht.__webglTexture=t.createTexture();ht.__version=_.version,a.memory.textures++}if(mt){B.__webglFramebuffer=[];for(let Y=0;Y<6;Y++)if(_.mipmaps.length>0){B.__webglFramebuffer[Y]=[];for(let et=0;et<_.mipmaps.length;et++)B.__webglFramebuffer[Y][et]=t.createFramebuffer()}else B.__webglFramebuffer[Y]=t.createFramebuffer()}else{if(_.mipmaps.length>0){B.__webglFramebuffer=[];for(let Y=0;Y<_.mipmaps.length;Y++)B.__webglFramebuffer[Y]=t.createFramebuffer()}else B.__webglFramebuffer=t.createFramebuffer();if(Mt)for(let Y=0,et=it.length;Y<et;Y++){let dt=i.get(it[Y]);if(dt.__webglTexture===void 0)dt.__webglTexture=t.createTexture(),a.memory.textures++}if(b.samples>0&&re(b)===!1){B.__webglMultisampledFramebuffer=t.createFramebuffer(),B.__webglColorRenderbuffer=[],n.bindFramebuffer(t.FRAMEBUFFER,B.__webglMultisampledFramebuffer);for(let Y=0;Y<it.length;Y++){let et=it[Y];B.__webglColorRenderbuffer[Y]=t.createRenderbuffer(),t.bindRenderbuffer(t.RENDERBUFFER,B.__webglColorRenderbuffer[Y]);let dt=r.convert(et.format,et.colorSpace),I=r.convert(et.type),Q=P(et.internalFormat,dt,I,et.normalized,et.colorSpace,b.isXRRenderTarget===!0),Z=at(b);t.renderbufferStorageMultisample(t.RENDERBUFFER,Z,Q,b.width,b.height),t.framebufferRenderbuffer(t.FRAMEBUFFER,t.COLOR_ATTACHMENT0+Y,t.RENDERBUFFER,B.__webglColorRenderbuffer[Y])}if(t.bindRenderbuffer(t.RENDERBUFFER,null),b.depthBuffer)B.__webglDepthRenderbuffer=t.createRenderbuffer(),be(B.__webglDepthRenderbuffer,b,!0);n.bindFramebuffer(t.FRAMEBUFFER,null)}}if(mt){n.bindTexture(t.TEXTURE_CUBE_MAP,ht.__webglTexture),ve(t.TEXTURE_CUBE_MAP,_);for(let Y=0;Y<6;Y++)if(_.mipmaps.length>0)for(let et=0;et<_.mipmaps.length;et++)_e(B.__webglFramebuffer[Y][et],b,_,t.COLOR_ATTACHMENT0,t.TEXTURE_CUBE_MAP_POSITIVE_X+Y,et);else _e(B.__webglFramebuffer[Y],b,_,t.COLOR_ATTACHMENT0,t.TEXTURE_CUBE_MAP_POSITIVE_X+Y,0);if(T(_))S(t.TEXTURE_CUBE_MAP);else if(_.mipmaps.length>0)t.texParameteri(t.TEXTURE_CUBE_MAP,t.TEXTURE_MAX_LEVEL,_.mipmaps.length-1);n.unbindTexture()}else if(Mt){for(let Y=0,et=it.length;Y<et;Y++){let dt=it[Y],I=i.get(dt),Q=t.TEXTURE_2D;if(b.isWebGL3DRenderTarget||b.isWebGLArrayRenderTarget)Q=b.isWebGL3DRenderTarget?t.TEXTURE_3D:t.TEXTURE_2D_ARRAY;if(n.bindTexture(Q,I.__webglTexture),ve(Q,dt),_e(B.__webglFramebuffer,b,dt,t.COLOR_ATTACHMENT0+Y,Q,0),T(dt))S(Q)}n.unbindTexture()}else{let Y=t.TEXTURE_2D;if(b.isWebGL3DRenderTarget||b.isWebGLArrayRenderTarget)Y=b.isWebGL3DRenderTarget?t.TEXTURE_3D:t.TEXTURE_2D_ARRAY;if(n.bindTexture(Y,ht.__webglTexture),ve(Y,_),_.mipmaps.length>0)for(let et=0;et<_.mipmaps.length;et++)_e(B.__webglFramebuffer[et],b,_,t.COLOR_ATTACHMENT0,Y,et);else _e(B.__webglFramebuffer,b,_,t.COLOR_ATTACHMENT0,Y,0);if(T(_))S(Y);else if(_.mipmaps.length>0)t.texParameteri(Y,t.TEXTURE_MAX_LEVEL,_.mipmaps.length-1);n.unbindTexture()}if(b.depthBuffer)Fe(b)}function Ye(b){let _=b.textures;for(let B=0,ht=_.length;B<ht;B++){let it=_[B];if(T(it)&&it.mipmapsAutoUpdate===!0){let mt=E(b),Mt=i.get(it).__webglTexture;n.bindTexture(mt,Mt),S(mt),n.unbindTexture()}}}let de=[];function xe(b){if(b.samples>0){if(re(b)===!1){let{textures:_,width:B,height:ht}=b,it=t.COLOR_BUFFER_BIT,mt=b.stencilBuffer?t.DEPTH_STENCIL_ATTACHMENT:t.DEPTH_ATTACHMENT,Mt=i.get(b),Y=_.length>1;if(Y)for(let dt=0;dt<_.length;dt++)n.bindFramebuffer(t.FRAMEBUFFER,Mt.__webglMultisampledFramebuffer),t.framebufferRenderbuffer(t.FRAMEBUFFER,t.COLOR_ATTACHMENT0+dt,t.RENDERBUFFER,null),n.bindFramebuffer(t.FRAMEBUFFER,Mt.__webglFramebuffer),t.framebufferTexture2D(t.DRAW_FRAMEBUFFER,t.COLOR_ATTACHMENT0+dt,t.TEXTURE_2D,null,0);if(n.bindFramebuffer(t.READ_FRAMEBUFFER,Mt.__webglMultisampledFramebuffer),b.texture.mipmaps.length>0)n.bindFramebuffer(t.DRAW_FRAMEBUFFER,Mt.__webglFramebuffer[0]);else n.bindFramebuffer(t.DRAW_FRAMEBUFFER,Mt.__webglFramebuffer);for(let dt=0;dt<_.length;dt++){if(b.resolveDepthBuffer){if(b.depthBuffer)it|=t.DEPTH_BUFFER_BIT;if(b.stencilBuffer&&b.resolveStencilBuffer)it|=t.STENCIL_BUFFER_BIT}if(Y){t.framebufferRenderbuffer(t.READ_FRAMEBUFFER,t.COLOR_ATTACHMENT0,t.RENDERBUFFER,Mt.__webglColorRenderbuffer[dt]);let I=i.get(_[dt]).__webglTexture;t.framebufferTexture2D(t.DRAW_FRAMEBUFFER,t.COLOR_ATTACHMENT0,t.TEXTURE_2D,I,0)}if(t.blitFramebuffer(0,0,B,ht,0,0,B,ht,it,t.NEAREST),l===!0){if(de.length=0,de.push(t.COLOR_ATTACHMENT0+dt),b.depthBuffer&&b.storeMultisampledDepthBuffer===!1)de.push(mt);t.invalidateFramebuffer(t.READ_FRAMEBUFFER,de)}}if(n.bindFramebuffer(t.READ_FRAMEBUFFER,null),n.bindFramebuffer(t.DRAW_FRAMEBUFFER,null),Y)for(let dt=0;dt<_.length;dt++){n.bindFramebuffer(t.FRAMEBUFFER,Mt.__webglMultisampledFramebuffer),t.framebufferRenderbuffer(t.FRAMEBUFFER,t.COLOR_ATTACHMENT0+dt,t.RENDERBUFFER,Mt.__webglColorRenderbuffer[dt]);let I=i.get(_[dt]).__webglTexture;n.bindFramebuffer(t.FRAMEBUFFER,Mt.__webglFramebuffer),t.framebufferTexture2D(t.DRAW_FRAMEBUFFER,t.COLOR_ATTACHMENT0+dt,t.TEXTURE_2D,I,0)}n.bindFramebuffer(t.DRAW_FRAMEBUFFER,Mt.__webglMultisampledFramebuffer)}else if(b.depthBuffer&&b.storeMultisampledDepthBuffer===!1&&l){let _=b.stencilBuffer?t.DEPTH_STENCIL_ATTACHMENT:t.DEPTH_ATTACHMENT;t.invalidateFramebuffer(t.DRAW_FRAMEBUFFER,[_])}}}function at(b){return Math.min(s.maxSamples,b.samples)}function re(b){let _=i.get(b);return b.samples>0&&e.has("WEBGL_multisampled_render_to_texture")===!0&&_.__useRenderToTexture!==!1}function kt(b){let _=a.render.frame;if(p.get(b)!==_)p.set(b,_),b.update()}function Xt(b,_){let{colorSpace:B,format:ht,type:it}=b;if(b.isCompressedTexture===!0||b.isVideoTexture===!0)return _;if(B!==ja&&B!==hi)if(ae.getTransfer(B)===ye){if(ht!==Fn||it!==xn)Wt("WebGLTextures: sRGB encoded textures have to use RGBAFormat and UnsignedByteType.")}else Yt("WebGLTextures: Unsupported texture color space:",B);return _}function Oe(b){if(typeof HTMLImageElement<"u"&&b instanceof HTMLImageElement)c.width=b.naturalWidth||b.width,c.height=b.naturalHeight||b.height;else if(typeof VideoFrame<"u"&&b instanceof VideoFrame)c.width=b.displayWidth,c.height=b.displayHeight;else c.width=b.width,c.height=b.height;return c}this.allocateTextureUnit=nt,this.resetTextureUnits=st,this.getTextureUnits=k,this.setTextureUnits=It,this.setTexture2D=yt,this.setTexture2DArray=Gt,this.setTexture3D=zt,this.setTextureCube=Dt,this.rebindTextures=Je,this.setupRenderTarget=U,this.updateRenderTargetMipmap=Ye,this.updateMultisampleRenderTarget=xe,this.setupDepthRenderbuffer=Fe,this.setupFrameBufferTexture=_e,this.useMultisampledRTT=re,this.dispose=j}function km(t,e){function n(i,s=hi){let r,a=ae.getTransfer(s);if(i===xn)return t.UNSIGNED_BYTE;if(i===_a)return t.UNSIGNED_SHORT_4_4_4_4;if(i===xa)return t.UNSIGNED_SHORT_5_5_5_1;if(i===ac)return t.UNSIGNED_INT_5_9_9_9_REV;if(i===oc)return t.UNSIGNED_INT_10F_11F_11F_REV;if(i===sc)return t.BYTE;if(i===rc)return t.SHORT;if(i===os)return t.UNSIGNED_SHORT;if(i===ga)return t.INT;if(i===Zn)return t.UNSIGNED_INT;if(i===Un)return t.FLOAT;if(i===bn)return t.HALF_FLOAT;if(i===lc)return t.ALPHA;if(i===cc)return t.RGB;if(i===Fn)return t.RGBA;if(i===Gi)return t.DEPTH_COMPONENT;if(i===Jn)return t.DEPTH_STENCIL;if(i===hc)return t.RED;if(i===va)return t.RED_INTEGER;if(i===ci)return t.RG;if(i===ya)return t.RG_INTEGER;if(i===Ma)return t.RGBA_INTEGER;if(i===ir||i===sr||i===rr||i===ar)if(a===ye)if(r=e.get("WEBGL_compressed_texture_s3tc_srgb"),r!==null){if(i===ir)return r.COMPRESSED_SRGB_S3TC_DXT1_EXT;if(i===sr)return r.COMPRESSED_SRGB_ALPHA_S3TC_DXT1_EXT;if(i===rr)return r.COMPRESSED_SRGB_ALPHA_S3TC_DXT3_EXT;if(i===ar)return r.COMPRESSED_SRGB_ALPHA_S3TC_DXT5_EXT}else return null;else if(r=e.get("WEBGL_compressed_texture_s3tc"),r!==null){if(i===ir)return r.COMPRESSED_RGB_S3TC_DXT1_EXT;if(i===sr)return r.COMPRESSED_RGBA_S3TC_DXT1_EXT;if(i===rr)return r.COMPRESSED_RGBA_S3TC_DXT3_EXT;if(i===ar)return r.COMPRESSED_RGBA_S3TC_DXT5_EXT}else return null;if(i===Sa||i===ba||i===wa||i===Aa)if(r=e.get("WEBGL_compressed_texture_pvrtc"),r!==null){if(i===Sa)return r.COMPRESSED_RGB_PVRTC_4BPPV1_IMG;if(i===ba)return r.COMPRESSED_RGB_PVRTC_2BPPV1_IMG;if(i===wa)return r.COMPRESSED_RGBA_PVRTC_4BPPV1_IMG;if(i===Aa)return r.COMPRESSED_RGBA_PVRTC_2BPPV1_IMG}else return null;if(i===Ta||i===Ea||i===Ra||i===Ca||i===Ia||i===or||i===Pa)if(r=e.get("WEBGL_compressed_texture_etc"),r!==null){if(i===Ta||i===Ea)return a===ye?r.COMPRESSED_SRGB8_ETC2:r.COMPRESSED_RGB8_ETC2;if(i===Ra)return a===ye?r.COMPRESSED_SRGB8_ALPHA8_ETC2_EAC:r.COMPRESSED_RGBA8_ETC2_EAC;if(i===Ca)return r.COMPRESSED_R11_EAC;if(i===Ia)return r.COMPRESSED_SIGNED_R11_EAC;if(i===or)return r.COMPRESSED_RG11_EAC;if(i===Pa)return r.COMPRESSED_SIGNED_RG11_EAC}else return null;if(i===La||i===Na||i===Da||i===Ua||i===Fa||i===Oa||i===Ba||i===za||i===ka||i===Ga||i===Ha||i===Wa||i===Va||i===Xa)if(r=e.get("WEBGL_compressed_texture_astc"),r!==null){if(i===La)return a===ye?r.COMPRESSED_SRGB8_ALPHA8_ASTC_4x4_KHR:r.COMPRESSED_RGBA_ASTC_4x4_KHR;if(i===Na)return a===ye?r.COMPRESSED_SRGB8_ALPHA8_ASTC_5x4_KHR:r.COMPRESSED_RGBA_ASTC_5x4_KHR;if(i===Da)return a===ye?r.COMPRESSED_SRGB8_ALPHA8_ASTC_5x5_KHR:r.COMPRESSED_RGBA_ASTC_5x5_KHR;if(i===Ua)return a===ye?r.COMPRESSED_SRGB8_ALPHA8_ASTC_6x5_KHR:r.COMPRESSED_RGBA_ASTC_6x5_KHR;if(i===Fa)return a===ye?r.COMPRESSED_SRGB8_ALPHA8_ASTC_6x6_KHR:r.COMPRESSED_RGBA_ASTC_6x6_KHR;if(i===Oa)return a===ye?r.COMPRESSED_SRGB8_ALPHA8_ASTC_8x5_KHR:r.COMPRESSED_RGBA_ASTC_8x5_KHR;if(i===Ba)return a===ye?r.COMPRESSED_SRGB8_ALPHA8_ASTC_8x6_KHR:r.COMPRESSED_RGBA_ASTC_8x6_KHR;if(i===za)return a===ye?r.COMPRESSED_SRGB8_ALPHA8_ASTC_8x8_KHR:r.COMPRESSED_RGBA_ASTC_8x8_KHR;if(i===ka)return a===ye?r.COMPRESSED_SRGB8_ALPHA8_ASTC_10x5_KHR:r.COMPRESSED_RGBA_ASTC_10x5_KHR;if(i===Ga)return a===ye?r.COMPRESSED_SRGB8_ALPHA8_ASTC_10x6_KHR:r.COMPRESSED_RGBA_ASTC_10x6_KHR;if(i===Ha)return a===ye?r.COMPRESSED_SRGB8_ALPHA8_ASTC_10x8_KHR:r.COMPRESSED_RGBA_ASTC_10x8_KHR;if(i===Wa)return a===ye?r.COMPRESSED_SRGB8_ALPHA8_ASTC_10x10_KHR:r.COMPRESSED_RGBA_ASTC_10x10_KHR;if(i===Va)return a===ye?r.COMPRESSED_SRGB8_ALPHA8_ASTC_12x10_KHR:r.COMPRESSED_RGBA_ASTC_12x10_KHR;if(i===Xa)return a===ye?r.COMPRESSED_SRGB8_ALPHA8_ASTC_12x12_KHR:r.COMPRESSED_RGBA_ASTC_12x12_KHR}else return null;if(i===qa||i===Ya||i===Za)if(r=e.get("EXT_texture_compression_bptc"),r!==null){if(i===qa)return a===ye?r.COMPRESSED_SRGB_ALPHA_BPTC_UNORM_EXT:r.COMPRESSED_RGBA_BPTC_UNORM_EXT;if(i===Ya)return r.COMPRESSED_RGB_BPTC_SIGNED_FLOAT_EXT;if(i===Za)return r.COMPRESSED_RGB_BPTC_UNSIGNED_FLOAT_EXT}else return null;if(i===Ja||i===$a||i===lr||i===Ka)if(r=e.get("EXT_texture_compression_rgtc"),r!==null){if(i===Ja)return r.COMPRESSED_RED_RGTC1_EXT;if(i===$a)return r.COMPRESSED_SIGNED_RED_RGTC1_EXT;if(i===lr)return r.COMPRESSED_RED_GREEN_RGTC2_EXT;if(i===Ka)return r.COMPRESSED_SIGNED_RED_GREEN_RGTC2_EXT}else return null;if(i===li)return t.UNSIGNED_INT_24_8;return t[i]!==void 0?t[i]:null}return{convert:n}}var Gm=`
void main() {

	gl_Position = vec4( position, 1.0 );

}`,Hm=`
uniform sampler2DArray depthColor;
uniform float depthWidth;
uniform float depthHeight;

void main() {

	vec2 coord = vec2( gl_FragCoord.x / depthWidth, gl_FragCoord.y / depthHeight );

	if ( coord.x >= 1.0 ) {

		gl_FragDepth = texture( depthColor, vec3( coord.x - 1.0, coord.y, 1 ) ).r;

	} else {

		gl_FragDepth = texture( depthColor, vec3( coord.x, coord.y, 0 ) ).r;

	}

}`;class uh{constructor(){this.texture=null,this.mesh=null,this.depthNear=0,this.depthFar=0}init(t,e){if(this.texture===null){let n=new vr(t.texture);if(t.depthNear!==e.depthNear||t.depthFar!==e.depthFar)this.depthNear=t.depthNear,this.depthFar=t.depthFar;this.texture=n}}getMesh(t){if(this.texture!==null){if(this.mesh===null){let e=t.cameras[0].viewport,n=new vn({vertexShader:Gm,fragmentShader:Hm,uniforms:{depthColor:{value:this.texture},depthWidth:{value:e.z},depthHeight:{value:e.w}}});this.mesh=new hn(new ds(20,20),n)}}return this.mesh}reset(){this.texture=null,this.mesh=null}getDepthTexture(){return this.texture}}class dh extends On{constructor(t,e){super();let n=this,i=null,s=1,r=null,a="local-floor",o=1,l=null,c=null,p=null,h=null,u=null,d=null,v=typeof XRWebGLBinding<"u",w=new uh,f={},m=e.getContextAttributes(),M=null,T=null,S=[],E=[],P=new fe,g=null,y=null,D=new $e;D.viewport=new Ae;let q=new $e;q.viewport=new Ae;let N=[D,q],X=new Lo,rt=null,R=null;this.cameraAutoUpdate=!0,this.enabled=!1,this.isPresenting=!1,this.getController=function(nt){let ct=S[nt];if(ct===void 0)ct=new hs,S[nt]=ct;return ct.getTargetRaySpace()},this.getControllerGrip=function(nt){let ct=S[nt];if(ct===void 0)ct=new hs,S[nt]=ct;return ct.getGripSpace()},this.getHand=function(nt){let ct=S[nt];if(ct===void 0)ct=new hs,S[nt]=ct;return ct.getHandSpace()};function j(nt){let ct=E.indexOf(nt.inputSource);if(ct===-1)return;let yt=S[ct];if(yt!==void 0)yt.update(nt.inputSource,nt.frame,l||r),yt.dispatchEvent({type:nt.type,data:nt.inputSource})}function tt(){i.removeEventListener("select",j),i.removeEventListener("selectstart",j),i.removeEventListener("selectend",j),i.removeEventListener("squeeze",j),i.removeEventListener("squeezestart",j),i.removeEventListener("squeezeend",j),i.removeEventListener("end",tt),i.removeEventListener("inputsourceschange",G);for(let nt=0;nt<S.length;nt++){let ct=E[nt];if(ct===null)continue;E[nt]=null,S[nt].disconnect(ct)}rt=null,R=null,w.reset();for(let nt in f)delete f[nt];if(t.setRenderTarget(M),u=null,h=null,p=null,i=null,T=null,It.stop(),n.isPresenting=!1,t.setPixelRatio(g),t.setSize(P.width,P.height,!1),y!==null){let nt=y.camera;nt.fov=y.fov,nt.zoom=y.zoom,nt.updateProjectionMatrix(),y=null}n.dispatchEvent({type:"sessionend"})}this.setFramebufferScaleFactor=function(nt){if(s=nt,n.isPresenting===!0)Wt("WebXRManager: Cannot change framebuffer scale while presenting.")},this.setReferenceSpaceType=function(nt){if(a=nt,n.isPresenting===!0)Wt("WebXRManager: Cannot change reference space type while presenting.")},this.getReferenceSpace=function(){return l||r},this.setReferenceSpace=function(nt){l=nt},this.getBaseLayer=function(){return h!==null?h:u},this.getBinding=function(){if(p===null&&v)p=new XRWebGLBinding(i,e);return p},this.getFrame=function(){return d},this.getSession=function(){return i},this.setSession=async function(nt){if(i=nt,i!==null){if(M=t.getRenderTarget(),i.addEventListener("select",j),i.addEventListener("selectstart",j),i.addEventListener("selectend",j),i.addEventListener("squeeze",j),i.addEventListener("squeezestart",j),i.addEventListener("squeezeend",j),i.addEventListener("end",tt),i.addEventListener("inputsourceschange",G),m.xrCompatible!==!0)await e.makeXRCompatible();if(g=t.getPixelRatio(),t.getSize(P),!(v&&("createProjectionLayer"in XRWebGLBinding.prototype))){let yt={antialias:m.antialias,alpha:!0,depth:m.depth,stencil:m.stencil,framebufferScaleFactor:s};u=new XRWebGLLayer(i,e,yt),i.updateRenderState({baseLayer:u}),t.setPixelRatio(1),t.setSize(u.framebufferWidth,u.framebufferHeight,!1),T=new cn(u.framebufferWidth,u.framebufferHeight,{format:Fn,type:xn,colorSpace:t.outputColorSpace,stencilBuffer:m.stencil,resolveDepthBuffer:u.ignoreDepthValues===!1,resolveStencilBuffer:u.ignoreDepthValues===!1,storeMultisampledDepthBuffer:u.ignoreDepthValues===!1,storeMultisampledStencilBuffer:u.ignoreDepthValues===!1})}else{let yt=null,Gt=null,zt=null;if(m.depth)zt=m.stencil?e.DEPTH24_STENCIL8:e.DEPTH_COMPONENT24,yt=m.stencil?Jn:Gi,Gt=m.stencil?li:Zn;let Dt={colorFormat:e.RGBA8,depthFormat:zt,scaleFactor:s};p=this.getBinding(),h=p.createProjectionLayer(Dt),i.updateRenderState({layers:[h]}),t.setPixelRatio(1),t.setSize(h.textureWidth,h.textureHeight,!1),T=new cn(h.textureWidth,h.textureHeight,{format:Fn,type:xn,depthTexture:new Kn(h.textureWidth,h.textureHeight,Gt,void 0,void 0,void 0,void 0,void 0,void 0,yt),stencilBuffer:m.stencil,colorSpace:t.outputColorSpace,samples:m.antialias?4:0,resolveDepthBuffer:h.ignoreDepthValues===!1,resolveStencilBuffer:h.ignoreDepthValues===!1,storeMultisampledDepthBuffer:h.ignoreDepthValues===!1,storeMultisampledStencilBuffer:h.ignoreDepthValues===!1})}T.isXRRenderTarget=!0,this.setFoveation(o),l=null,r=await i.requestReferenceSpace(a),It.setContext(i),It.start(),n.isPresenting=!0,n.dispatchEvent({type:"sessionstart"})}},this.getEnvironmentBlendMode=function(){if(i!==null)return i.environmentBlendMode},this.getDepthTexture=function(){return w.getDepthTexture()};function G(nt){for(let ct=0;ct<nt.removed.length;ct++){let yt=nt.removed[ct],Gt=E.indexOf(yt);if(Gt>=0)E[Gt]=null,S[Gt].disconnect(yt)}for(let ct=0;ct<nt.added.length;ct++){let yt=nt.added[ct],Gt=E.indexOf(yt);if(Gt===-1){for(let Dt=0;Dt<S.length;Dt++)if(Dt>=E.length){E.push(yt),Gt=Dt;break}else if(E[Dt]===null){E[Dt]=yt,Gt=Dt;break}if(Gt===-1)break}let zt=S[Gt];if(zt)zt.connect(yt)}}function pt(nt,ct){if(ct===null)nt.matrixWorld.copy(nt.matrix);else nt.matrixWorld.multiplyMatrices(ct.matrixWorld,nt.matrix);nt.matrixWorldInverse.copy(nt.matrixWorld).invert()}this.updateCamera=function(nt){if(i===null)return;let{near:ct,far:yt}=nt;if(w.texture!==null){if(w.depthNear>0)ct=w.depthNear;if(w.depthFar>0)yt=w.depthFar}if(X.near=q.near=D.near=ct,X.far=q.far=D.far=yt,rt!==X.near||R!==X.far)i.updateRenderState({depthNear:X.near,depthFar:X.far}),rt=X.near,R=X.far;X.layers.mask=nt.layers.mask|6,D.layers.mask=X.layers.mask&-5,q.layers.mask=X.layers.mask&-3;let Gt=nt.parent,zt=X.cameras;pt(X,Gt);for(let Dt=0;Dt<zt.length;Dt++)pt(zt[Dt],Gt);if(X.projectionMatrix.copy(D.projectionMatrix),X.projectionMatrixInverse.copy(D.projectionMatrixInverse),y===null&&nt.isPerspectiveCamera)y={camera:nt,fov:nt.fov,zoom:nt.zoom};J(nt,X,Gt)};function J(nt,ct,yt){if(yt===null)nt.matrix.copy(ct.matrixWorld);else nt.matrix.copy(yt.matrixWorld),nt.matrix.invert(),nt.matrix.multiply(ct.matrixWorld);if(nt.matrix.decompose(nt.position,nt.quaternion,nt.scale),nt.updateMatrixWorld(!0),nt.projectionMatrix.copy(ct.projectionMatrix),nt.projectionMatrixInverse.copy(ct.projectionMatrixInverse),nt.isPerspectiveCamera)nt.fov=$s*2*Math.atan(1/nt.projectionMatrix.elements[5]),nt.zoom=1}this.getCamera=function(){return X},this.getFoveation=function(){if(h===null&&u===null)return;return o},this.setFoveation=function(nt){if(o=nt,h!==null)h.fixedFoveation=nt;if(u!==null&&u.fixedFoveation!==void 0)u.fixedFoveation=nt},this.hasDepthSensing=function(){return w.texture!==null},this.getDepthSensingMesh=function(){return w.getMesh(X)},this.getCameraTexture=function(nt){return f[nt]};let st=null;function k(nt,ct){if(c=ct.getViewerPose(l||r),d=ct,c!==null){let yt=c.views;if(u!==null)t.setRenderTargetFramebuffer(T,u.framebuffer),t.setRenderTarget(T);let Gt=!1;if(yt.length!==X.cameras.length)X.cameras.length=0,Gt=!0;for(let ee=0;ee<yt.length;ee++){let Jt=yt[ee],ve=null;if(u!==null)ve=u.getViewport(Jt);else{let Me=p.getViewSubImage(h,Jt);if(ve=Me.viewport,ee===0)t.setRenderTargetTextures(T,Me.colorTexture,Me.depthStencilTexture),t.setRenderTarget(T)}let ie=N[ee];if(ie===void 0)ie=new $e,ie.layers.enable(ee),ie.viewport=new Ae,N[ee]=ie;if(ie.matrix.fromArray(Jt.transform.matrix),ie.matrix.decompose(ie.position,ie.quaternion,ie.scale),ie.projectionMatrix.fromArray(Jt.projectionMatrix),ie.projectionMatrixInverse.copy(ie.projectionMatrix).invert(),ie.viewport.set(ve.x,ve.y,ve.width,ve.height),ee===0)X.matrix.copy(ie.matrix),X.matrix.decompose(X.position,X.quaternion,X.scale);if(Gt===!0)X.cameras.push(ie)}let zt=i.enabledFeatures;if(zt&&zt.includes("depth-sensing")&&i.depthUsage=="gpu-optimized"&&v){p=n.getBinding();let ee=p.getDepthInformation(yt[0]);if(ee&&ee.isValid&&ee.texture)w.init(ee,i.renderState)}if(zt&&zt.includes("camera-access")&&v){t.state.unbindTexture(),p=n.getBinding();for(let ee=0;ee<yt.length;ee++){let Jt=yt[ee].camera;if(Jt){let ve=f[Jt];if(!ve)ve=new vr,f[Jt]=ve;let ie=p.getCameraImage(Jt);ve.sourceTexture=ie}}}}for(let yt=0;yt<S.length;yt++){let Gt=E[yt],zt=S[yt];if(Gt!==null&&zt!==void 0)zt.update(Gt,ct,l||r)}if(st)st(nt,ct);if(ct.detectedPlanes)n.dispatchEvent({type:"planesdetected",data:ct});d=null}let It=new $c;It.setAnimationLoop(k),this.setAnimationLoop=function(nt){st=nt},this.dispose=function(){}}}var Wm=new Se,fh=new Zt;fh.set(-1,0,0,0,1,0,0,0,1);function Vm(t,e){function n(f,m){if(f.matrixAutoUpdate===!0)f.updateMatrix();m.value.copy(f.matrix)}function i(f,m){if(m.color.getRGB(f.fogColor.value,_o(t)),m.isFog)f.fogNear.value=m.near,f.fogFar.value=m.far;else if(m.isFogExp2)f.fogDensity.value=m.density}function s(f,m,M,T,S){if(m.isNodeMaterial)m.uniformsNeedUpdate=!1;else if(m.isMeshBasicMaterial)r(f,m);else if(m.isMeshLambertMaterial){if(r(f,m),m.envMap)f.envMapIntensity.value=m.envMapIntensity}else if(m.isMeshToonMaterial)r(f,m),h(f,m);else if(m.isMeshPhongMaterial){if(r(f,m),p(f,m),m.envMap)f.envMapIntensity.value=m.envMapIntensity}else if(m.isMeshStandardMaterial){if(r(f,m),u(f,m),m.isMeshPhysicalMaterial)d(f,m,S)}else if(m.isMeshMatcapMaterial)r(f,m),v(f,m);else if(m.isMeshDepthMaterial)r(f,m);else if(m.isMeshDistanceMaterial)r(f,m),w(f,m);else if(m.isMeshNormalMaterial)r(f,m);else if(m.isLineBasicMaterial){if(a(f,m),m.isLineDashedMaterial)o(f,m)}else if(m.isPointsMaterial)l(f,m,M,T);else if(m.isSpriteMaterial)c(f,m);else if(m.isShadowMaterial)f.color.value.copy(m.color),f.opacity.value=m.opacity;else if(m.isShaderMaterial)m.uniformsNeedUpdate=!1}function r(f,m){if(f.opacity.value=m.opacity,m.color)f.diffuse.value.copy(m.color);if(m.emissive)f.emissive.value.copy(m.emissive).multiplyScalar(m.emissiveIntensity);if(m.map)f.map.value=m.map,n(m.map,f.mapTransform);if(m.alphaMap)f.alphaMap.value=m.alphaMap,n(m.alphaMap,f.alphaMapTransform);if(m.bumpMap){if(f.bumpMap.value=m.bumpMap,n(m.bumpMap,f.bumpMapTransform),f.bumpScale.value=m.bumpScale,m.side===Qe)f.bumpScale.value*=-1}if(m.normalMap){if(f.normalMap.value=m.normalMap,n(m.normalMap,f.normalMapTransform),f.normalScale.value.copy(m.normalScale),m.side===Qe)f.normalScale.value.negate()}if(m.displacementMap)f.displacementMap.value=m.displacementMap,n(m.displacementMap,f.displacementMapTransform),f.displacementScale.value=m.displacementScale,f.displacementBias.value=m.displacementBias;if(m.emissiveMap)f.emissiveMap.value=m.emissiveMap,n(m.emissiveMap,f.emissiveMapTransform);if(m.specularMap)f.specularMap.value=m.specularMap,n(m.specularMap,f.specularMapTransform);if(m.alphaTest>0)f.alphaTest.value=m.alphaTest;let M=e.get(m),{envMap:T,envMapRotation:S}=M;if(T){if(f.envMap.value=T,f.envMapRotation.value.setFromMatrix4(Wm.makeRotationFromEuler(S)).transpose(),T.isCubeTexture&&T.isRenderTargetTexture===!1)f.envMapRotation.value.premultiply(fh);f.reflectivity.value=m.reflectivity,f.ior.value=m.ior,f.refractionRatio.value=m.refractionRatio}if(m.lightMap)f.lightMap.value=m.lightMap,f.lightMapIntensity.value=m.lightMapIntensity,n(m.lightMap,f.lightMapTransform);if(m.aoMap)f.aoMap.value=m.aoMap,f.aoMapIntensity.value=m.aoMapIntensity,n(m.aoMap,f.aoMapTransform)}function a(f,m){if(f.diffuse.value.copy(m.color),f.opacity.value=m.opacity,m.map)f.map.value=m.map,n(m.map,f.mapTransform)}function o(f,m){f.dashSize.value=m.dashSize,f.totalSize.value=m.dashSize+m.gapSize,f.scale.value=m.scale}function l(f,m,M,T){if(f.diffuse.value.copy(m.color),f.opacity.value=m.opacity,f.size.value=m.size*M,f.scale.value=T*0.5,m.map)f.map.value=m.map,n(m.map,f.uvTransform);if(m.alphaMap)f.alphaMap.value=m.alphaMap,n(m.alphaMap,f.alphaMapTransform);if(m.alphaTest>0)f.alphaTest.value=m.alphaTest}function c(f,m){if(f.diffuse.value.copy(m.color),f.opacity.value=m.opacity,f.rotation.value=m.rotation,m.map)f.map.value=m.map,n(m.map,f.mapTransform);if(m.alphaMap)f.alphaMap.value=m.alphaMap,n(m.alphaMap,f.alphaMapTransform);if(m.alphaTest>0)f.alphaTest.value=m.alphaTest}function p(f,m){f.specular.value.copy(m.specular),f.shininess.value=Math.max(m.shininess,0.0001)}function h(f,m){if(m.gradientMap)f.gradientMap.value=m.gradientMap}function u(f,m){if(f.metalness.value=m.metalness,m.metalnessMap)f.metalnessMap.value=m.metalnessMap,n(m.metalnessMap,f.metalnessMapTransform);if(f.roughness.value=m.roughness,m.roughnessMap)f.roughnessMap.value=m.roughnessMap,n(m.roughnessMap,f.roughnessMapTransform);if(m.envMap)f.envMapIntensity.value=m.envMapIntensity}function d(f,m,M){if(f.ior.value=m.ior,m.diffuseRoughness>0){if(f.diffuseRoughness.value=m.diffuseRoughness,m.diffuseRoughnessMap)f.diffuseRoughnessMap.value=m.diffuseRoughnessMap,n(m.diffuseRoughnessMap,f.diffuseRoughnessMapTransform)}if(m.sheen>0){if(f.sheenColor.value.copy(m.sheenColor).multiplyScalar(m.sheen),f.sheenRoughness.value=m.sheenRoughness,m.sheenColorMap)f.sheenColorMap.value=m.sheenColorMap,n(m.sheenColorMap,f.sheenColorMapTransform);if(m.sheenRoughnessMap)f.sheenRoughnessMap.value=m.sheenRoughnessMap,n(m.sheenRoughnessMap,f.sheenRoughnessMapTransform)}if(m.clearcoat>0){if(f.clearcoat.value=m.clearcoat,f.clearcoatRoughness.value=m.clearcoatRoughness,m.clearcoatMap)f.clearcoatMap.value=m.clearcoatMap,n(m.clearcoatMap,f.clearcoatMapTransform);if(m.clearcoatRoughnessMap)f.clearcoatRoughnessMap.value=m.clearcoatRoughnessMap,n(m.clearcoatRoughnessMap,f.clearcoatRoughnessMapTransform);if(m.clearcoatNormalMap){if(f.clearcoatNormalMap.value=m.clearcoatNormalMap,n(m.clearcoatNormalMap,f.clearcoatNormalMapTransform),f.clearcoatNormalScale.value.copy(m.clearcoatNormalScale),m.side===Qe)f.clearcoatNormalScale.value.negate()}}if(m.dispersion>0)f.dispersion.value=m.dispersion;if(m.retroreflectivity>0)f.retroreflectivity.value=m.retroreflectivity;if(m.iridescence>0){if(f.iridescence.value=m.iridescence,f.iridescenceIOR.value=m.iridescenceIOR,f.iridescenceThicknessMinimum.value=m.iridescenceThicknessMinimum,f.iridescenceThicknessMaximum.value=m.iridescenceThicknessMaximum,m.iridescenceMap)f.iridescenceMap.value=m.iridescenceMap,n(m.iridescenceMap,f.iridescenceMapTransform);if(m.iridescenceThicknessMap)f.iridescenceThicknessMap.value=m.iridescenceThicknessMap,n(m.iridescenceThicknessMap,f.iridescenceThicknessMapTransform)}if(m.transmission>0){if(f.transmission.value=m.transmission,f.transmissionSamplerMap.value=M.texture,f.transmissionSamplerSize.value.set(M.width,M.height),m.transmissionMap)f.transmissionMap.value=m.transmissionMap,n(m.transmissionMap,f.transmissionMapTransform);if(f.thickness.value=m.thickness,m.thicknessMap)f.thicknessMap.value=m.thicknessMap,n(m.thicknessMap,f.thicknessMapTransform);f.attenuationDistance.value=m.attenuationDistance,f.attenuationColor.value.copy(m.attenuationColor)}if(m.anisotropy>0){if(f.anisotropyVector.value.set(m.anisotropy*Math.cos(m.anisotropyRotation),m.anisotropy*Math.sin(m.anisotropyRotation)),m.anisotropyMap)f.anisotropyMap.value=m.anisotropyMap,n(m.anisotropyMap,f.anisotropyMapTransform)}if(f.specularIntensity.value=m.specularIntensity,f.specularColor.value.copy(m.specularColor),m.specularColorMap)f.specularColorMap.value=m.specularColorMap,n(m.specularColorMap,f.specularColorMapTransform);if(m.specularIntensityMap)f.specularIntensityMap.value=m.specularIntensityMap,n(m.specularIntensityMap,f.specularIntensityMapTransform)}function v(f,m){if(m.matcap)f.matcap.value=m.matcap}function w(f,m){let M=e.get(m).light;f.referencePosition.value.setFromMatrixPosition(M.matrixWorld),f.nearDistance.value=M.shadow.camera.near,f.farDistance.value=M.shadow.camera.far}return{refreshFogUniforms:i,refreshMaterialUniforms:s}}function Xm(t,e,n,i){let s={},r={};function a(M,T){let S=T.program;i.uniformBlockBinding(M,S,s[M.id])}function o(M,T){let S=s[M.id];if(S===void 0)v(M),S=l(M),s[M.id]=S,M.addEventListener("dispose",f);let E=T.program;i.updateUBOMapping(M,E);let P=e.render.frame;if(r[M.id]!==P)c(M),r[M.id]=P}function l(M){let T=t.createBuffer(),{__size:S,usage:E}=M;return t.bindBuffer(t.UNIFORM_BUFFER,T),t.bufferData(t.UNIFORM_BUFFER,S,E),t.bindBuffer(t.UNIFORM_BUFFER,null),T}function c(M){let T=s[M.id],{uniforms:S,__cache:E}=M;t.bindBuffer(t.UNIFORM_BUFFER,T);for(let P=0,g=S.length;P<g;P++){let y=S[P];if(Array.isArray(y))for(let D=0,q=y.length;D<q;D++)p(y[D],P,D,E);else p(y,P,0,E)}t.bindBuffer(t.UNIFORM_BUFFER,null)}function p(M,T,S,E){if(u(M,T,S,E)===!0){let{__offset:P,value:g}=M;if(Array.isArray(g)){let y=0;for(let D=0;D<g.length;D++){let q=g[D],N=w(q,!0);h(q,M.__data,y),y+=N.storage/Float32Array.BYTES_PER_ELEMENT}}else h(g,M.__data,0);t.bufferSubData(t.UNIFORM_BUFFER,P,M.__data)}}function h(M,T,S){if(typeof M==="number"||typeof M==="boolean")T[S]=M;else if(M.isMatrix3)T[S]=M.elements[0],T[S+1]=M.elements[1],T[S+2]=M.elements[2],T[S+3]=0,T[S+4]=M.elements[3],T[S+5]=M.elements[4],T[S+6]=M.elements[5],T[S+7]=0,T[S+8]=M.elements[6],T[S+9]=M.elements[7],T[S+10]=M.elements[8],T[S+11]=0;else if(ArrayBuffer.isView(M))T.set(M.subarray(0,T.length-S),S);else M.toArray(T,S)}function u(M,T,S,E){let P=M.value,g=T+"_"+S;if(Array.isArray(P)){if(E[g]===void 0)E[g]=[];let y=E[g],D=!1;for(let q=0;q<P.length;q++)if(d(P[q],y,q)===!0)D=!0;return D}return d(P,E,g)}function d(M,T,S){if(T[S]===void 0){if(typeof M==="number"||typeof M==="boolean")T[S]=M;else if(ArrayBuffer.isView(M))T[S]=M.slice();else T[S]=M.clone();return!0}else{let E=T[S];if(typeof M==="number"||typeof M==="boolean"){if(E!==M)return T[S]=M,!0}else if(ArrayBuffer.isView(M))return!0;else if(E.equals(M)===!1)return E.copy(M),!0}return!1}function v(M){let T=M.uniforms,S=0,E=16;for(let g=0,y=T.length;g<y;g++){let D=Array.isArray(T[g])?T[g]:[T[g]];for(let q=0,N=D.length;q<N;q++){let X=D[q],rt=Array.isArray(X.value)?X.value:[X.value],R=Array.isArray(T[g])||Array.isArray(X.value),j=S;for(let G=0,pt=rt.length;G<pt;G++){let J=rt[G],st=w(J,R),k=S%E,It=k%st.boundary,nt=k+It;if(S+=It,nt!==0&&E-nt<st.storage)S+=E-nt;if(G===0)j=S;S+=st.storage}let tt=X.type==="int"?Int32Array:X.type==="uint"?Uint32Array:Float32Array;X.__data=new tt((S-j)/Float32Array.BYTES_PER_ELEMENT),X.__offset=j}}let P=S%E;if(P>0)S+=E-P;return M.__size=S,M.__cache={},this}function w(M,T){let S={boundary:0,storage:0};if(typeof M==="number"||typeof M==="boolean")S.boundary=4,S.storage=4;else if(M.isVector2)S.boundary=8,S.storage=8;else if(M.isVector3||M.isColor)S.boundary=16,S.storage=12;else if(M.isVector4)S.boundary=16,S.storage=16;else if(M.isMatrix3)S.boundary=16,S.storage=48;else if(M.isMatrix4)S.boundary=16,S.storage=64;else if(M.isTexture)Wt("WebGLRenderer: Texture samplers can not be part of an uniforms group.");else if(ArrayBuffer.isView(M))S.boundary=16,S.storage=M.byteLength;else Wt("WebGLRenderer: Unsupported uniform value type.",M);if(T===!0)S.boundary=16,S.storage=Math.ceil(S.storage/16)*16;return S}function f(M){let T=M.target;T.removeEventListener("dispose",f),t.deleteBuffer(s[T.id]),delete s[T.id],delete r[T.id]}function m(){for(let M in s)t.deleteBuffer(s[M]);s={},r={}}return{bind:a,update:o,dispose:m}}var qm=new Uint16Array([12469,15057,12620,14925,13266,14620,13807,14376,14323,13990,14545,13625,14713,13328,14840,12882,14931,12528,14996,12233,15039,11829,15066,11525,15080,11295,15085,10976,15082,10705,15073,10495,13880,14564,13898,14542,13977,14430,14158,14124,14393,13732,14556,13410,14702,12996,14814,12596,14891,12291,14937,11834,14957,11489,14958,11194,14943,10803,14921,10506,14893,10278,14858,9960,14484,14039,14487,14025,14499,13941,14524,13740,14574,13468,14654,13106,14743,12678,14818,12344,14867,11893,14889,11509,14893,11180,14881,10751,14852,10428,14812,10128,14765,9754,14712,9466,14764,13480,14764,13475,14766,13440,14766,13347,14769,13070,14786,12713,14816,12387,14844,11957,14860,11549,14868,11215,14855,10751,14825,10403,14782,10044,14729,9651,14666,9352,14599,9029,14967,12835,14966,12831,14963,12804,14954,12723,14936,12564,14917,12347,14900,11958,14886,11569,14878,11247,14859,10765,14828,10401,14784,10011,14727,9600,14660,9289,14586,8893,14508,8533,15111,12234,15110,12234,15104,12216,15092,12156,15067,12010,15028,11776,14981,11500,14942,11205,14902,10752,14861,10393,14812,9991,14752,9570,14682,9252,14603,8808,14519,8445,14431,8145,15209,11449,15208,11451,15202,11451,15190,11438,15163,11384,15117,11274,15055,10979,14994,10648,14932,10343,14871,9936,14803,9532,14729,9218,14645,8742,14556,8381,14461,8020,14365,7603,15273,10603,15272,10607,15267,10619,15256,10631,15231,10614,15182,10535,15118,10389,15042,10167,14963,9787,14883,9447,14800,9115,14710,8665,14615,8318,14514,7911,14411,7507,14279,7198,15314,9675,15313,9683,15309,9712,15298,9759,15277,9797,15229,9773,15166,9668,15084,9487,14995,9274,14898,8910,14800,8539,14697,8234,14590,7790,14479,7409,14367,7067,14178,6621,15337,8619,15337,8631,15333,8677,15325,8769,15305,8871,15264,8940,15202,8909,15119,8775,15022,8565,14916,8328,14804,8009,14688,7614,14569,7287,14448,6888,14321,6483,14088,6171,15350,7402,15350,7419,15347,7480,15340,7613,15322,7804,15287,7973,15229,8057,15148,8012,15046,7846,14933,7611,14810,7357,14682,7069,14552,6656,14421,6316,14251,5948,14007,5528,15356,5942,15356,5977,15353,6119,15348,6294,15332,6551,15302,6824,15249,7044,15171,7122,15070,7050,14949,6861,14818,6611,14679,6349,14538,6067,14398,5651,14189,5311,13935,4958,15359,4123,15359,4153,15356,4296,15353,4646,15338,5160,15311,5508,15263,5829,15188,6042,15088,6094,14966,6001,14826,5796,14678,5543,14527,5287,14377,4985,14133,4586,13869,4257,15360,1563,15360,1642,15358,2076,15354,2636,15341,3350,15317,4019,15273,4429,15203,4732,15105,4911,14981,4932,14836,4818,14679,4621,14517,4386,14359,4156,14083,3795,13808,3437,15360,122,15360,137,15358,285,15355,636,15344,1274,15322,2177,15281,2765,15215,3223,15120,3451,14995,3569,14846,3567,14681,3466,14511,3305,14344,3121,14037,2800,13753,2467,15360,0,15360,1,15359,21,15355,89,15346,253,15325,479,15287,796,15225,1148,15133,1492,15008,1749,14856,1882,14685,1886,14506,1783,14324,1608,13996,1398,13702,1183]),wn=null;function Ym(){if(wn===null)wn=new fo(qm,16,16,ci,bn),wn.name="DFG_LUT",wn.minFilter=ln,wn.magFilter=ln,wn.wrapS=er,wn.wrapT=er,wn.generateMipmaps=!1,wn.needsUpdate=!0;return wn}class zo{constructor(t={}){let{canvas:e=xc(),context:n=null,depth:i=!0,stencil:s=!1,alpha:r=!1,antialias:a=!1,premultipliedAlpha:o=!0,preserveDrawingBuffer:l=!1,powerPreference:c="default",failIfMajorPerformanceCaveat:p=!1,reversedDepthBuffer:h=!1,outputBufferType:u=xn}=t;this.isWebGLRenderer=!0;let d;if(n!==null){if(typeof WebGLRenderingContext<"u"&&n instanceof WebGLRenderingContext)throw Error("THREE.WebGLRenderer: WebGL 1 is not supported since r163.");d=n.getContextAttributes().alpha}else d=r;let v=u,w=new Set([Ma,ya,va]),f=new Set([xn,Zn,os,li,_a,xa]),m=new Uint32Array(4),M=new Int32Array(4),T={textures:[],min:[],max:[],resolution:[]},S=null,E=null,P=[],g=[],y=null;this.domElement=e,this.debug={checkShaderErrors:!0,diagnostics:{keywords:!1},onShaderError:null},this.autoClear=!0,this.autoClearColor=!0,this.autoClearDepth=!0,this.autoClearStencil=!0,this.sortObjects=!0,this.clippingPlanes=[],this.localClippingEnabled=!1,this.toneMapping=Sn,this.toneMappingExposure=1,this.transmissionResolutionScale=1;let D=this,q=!1,N=null,X=null,rt=null,R=null;this._outputColorSpace=dc;let j=0,tt=0,G=null,pt=-1,J=null,st=new Ae,k=new Ae,It=null,nt=new le(0),ct=0,{width:yt,height:Gt}=e,zt=1,Dt=null,ue=null,ee=new Ae(0,0,yt,Gt),Jt=new Ae(0,0,yt,Gt),ve=!1,ie=new Fi,Me=new mr,ge=!1,Re=!1,Le=new Se,_e=new W,be=new Ae,Ue={background:null,fog:null,environment:null,overrideMaterial:null,isScene:!0},Fe=!1;function Je(){return G===null?zt:1}let U=n;function Ye(x,L){return e.getContext(x,L)}let de,xe,at,re,kt,Xt,Oe,b,_,B,ht,it,mt,Mt,Y,et,dt,I,Q,Z,ut,ft,St;try{let x={alpha:!0,depth:i,stencil:s,antialias:a,premultipliedAlpha:o,preserveDrawingBuffer:l,powerPreference:c,failIfMajorPerformanceCaveat:p};if("setAttribute"in e)e.setAttribute("data-engine",`three.js r${Ml}`);if(e.addEventListener("webglcontextlost",Et,!1),e.addEventListener("webglcontextrestored",Ot,!1),e.addEventListener("webglcontextcreationerror",Vt,!1),U===null){if(U=Ye("webgl2",x),U===null)if(Ye("webgl2"))throw Error("THREE.WebGLRenderer: Error creating WebGL context with your selected attributes.");else throw Error("THREE.WebGLRenderer: Error creating WebGL context.")}Bt()}catch(x){throw e.removeEventListener("webglcontextlost",Et,!1),e.removeEventListener("webglcontextrestored",Ot,!1),e.removeEventListener("webglcontextcreationerror",Vt,!1),Yt("WebGLRenderer: "+x.message),x}function Bt(){if(de=new sp(U),de.init(),ut=new km(U,de),xe=new Yf(U,de,t,ut),at=new Bm(U,de),xe.reversedDepthBuffer&&h)at.buffers.depth.setReversed(!0);X=U.createFramebuffer(),rt=U.createFramebuffer(),R=U.createFramebuffer(),re=new op(U),kt=new wm,Xt=new zm(U,de,at,kt,xe,ut,re),Oe=new ip(D),b=new lu(U),ft=new Xf(U,b),_=new rp(U,b,re,ft),B=new cp(U,_,b,ft,re),I=new lp(U,xe,Xt),Y=new Zf(kt),ht=new bm(D,Oe,de,xe,ft,Y),it=new Vm(D,kt),mt=new Tm,Mt=new Lm(de),dt=new Vf(D,Oe,at,B,d,o),et=new Om(D,B,xe),St=new Xm(U,re,xe,at),Q=new qf(U,de,re),Z=new ap(U,de,re),re.programs=ht.programs,D.capabilities=xe,D.extensions=de,D.properties=kt,D.renderLists=mt,D.shadowMap=et,D.state=at,D.info=re}if(v!==xn)y=new up(v,e.width,e.height,a,i,s);let Ft=new dh(D,U);this.xr=Ft,this.getContext=function(){return U},this.getContextAttributes=function(){return U.getContextAttributes()},this.forceContextLoss=function(){let x=de.get("WEBGL_lose_context");if(x)x.loseContext()},this.forceContextRestore=function(){let x=de.get("WEBGL_lose_context");if(x)x.restoreContext()},this.getPixelRatio=function(){return zt},this.setPixelRatio=function(x){if(x===void 0)return;zt=x,this.setSize(yt,Gt,!1)},this.getSize=function(x){return x.set(yt,Gt)},this.setSize=function(x,L,V=!0){if(Ft.isPresenting){Wt("WebGLRenderer: Can't change size while VR device is presenting.");return}if(yt=x,Gt=L,e.width=Math.floor(x*zt),e.height=Math.floor(L*zt),V===!0)e.style.width=x+"px",e.style.height=L+"px";if(y!==null)y.setSize(e.width,e.height);this.setViewport(0,0,x,L)},this.getDrawingBufferSize=function(x){return x.set(yt*zt,Gt*zt).floor()},this.setDrawingBufferSize=function(x,L,V){yt=x,Gt=L,zt=V,e.width=Math.floor(x*V),e.height=Math.floor(L*V),this.setViewport(0,0,x,L)},this.setEffects=function(x){if(v===xn){Yt("WebGLRenderer: setEffects() requires outputBufferType set to HalfFloatType or FloatType.");return}if(x){for(let L=0;L<x.length;L++)if(x[L].isOutputPass===!0){Wt("WebGLRenderer: OutputPass is not needed in setEffects(). Tone mapping and color space conversion are applied automatically.");break}}y.setEffects(x||[])},this.getCurrentViewport=function(x){return x.copy(st)},this.getViewport=function(x){return x.copy(ee)},this.setViewport=function(x,L,V,z){if(x.isVector4)ee.set(x.x,x.y,x.z,x.w);else ee.set(x,L,V,z);at.viewport(st.copy(ee).multiplyScalar(Je()).round())},this.getScissor=function(x){return x.copy(Jt)},this.setScissor=function(x,L,V,z){if(x.isVector4)Jt.set(x.x,x.y,x.z,x.w);else Jt.set(x,L,V,z);at.scissor(k.copy(Jt).multiplyScalar(Je()).round())},this.getScissorTest=function(){return ve},this.setScissorTest=function(x){at.setScissorTest(ve=x)},this.setOpaqueSort=function(x){Dt=x},this.setTransparentSort=function(x){ue=x},this.getClearColor=function(x){return x.copy(dt.getClearColor())},this.setClearColor=function(){dt.setClearColor(...arguments)},this.getClearAlpha=function(){return dt.getClearAlpha()},this.setClearAlpha=function(){dt.setClearAlpha(...arguments)},this.clear=function(x=!0,L=!0,V=!0){let z=0;if(x){let H=!1;if(G!==null){let bt=G.texture.format;H=w.has(bt)}if(H){let bt=G.texture.type,Pt=f.has(bt),_t=dt.getClearColor(),A=dt.getClearAlpha(),{r:C,g:F,b:K}=_t;if(Pt)m[0]=C,m[1]=F,m[2]=K,m[3]=A,U.clearBufferuiv(U.COLOR,0,m);else M[0]=C,M[1]=F,M[2]=K,M[3]=A,U.clearBufferiv(U.COLOR,0,M)}else z|=U.COLOR_BUFFER_BIT}if(L)z|=U.DEPTH_BUFFER_BIT,this.state.buffers.depth.setMask(!0);if(V)z|=U.STENCIL_BUFFER_BIT,this.state.buffers.stencil.setMask(4294967295);if(z!==0)U.clear(z)},this.clearColor=function(){this.clear(!0,!1,!1)},this.clearDepth=function(){this.clear(!1,!0,!1)},this.clearStencil=function(){this.clear(!1,!1,!0)},this.setNodesHandler=function(x){x.setRenderer(this),N=x},this.dispose=function(){e.removeEventListener("webglcontextlost",Et,!1),e.removeEventListener("webglcontextrestored",Ot,!1),e.removeEventListener("webglcontextcreationerror",Vt,!1),dt.dispose(),mt.dispose(),Mt.dispose(),Xt.dispose(),kt.dispose(),Oe.dispose(),B.dispose(),_.dispose(),ft.dispose(),St.dispose(),ht.dispose(),Ft.dispose(),Ft.removeEventListener("sessionstart",jn),Ft.removeEventListener("sessionend",Ss),Tn.stop()};function Et(x){x.preventDefault(),ao("WebGLRenderer: Context Lost."),q=!0}function Ot(){ao("WebGLRenderer: Context Restored."),q=!1;let x=re.autoReset,L=et.enabled,V=et.autoUpdate,z=et.needsUpdate,H=et.type;Bt(),re.autoReset=x,et.enabled=L,et.autoUpdate=V,et.needsUpdate=z,et.type=H}function Vt(x){Yt("WebGLRenderer: A WebGL context could not be created. Reason: ",x.statusMessage)}function At(x){let L=x.target;L.removeEventListener("dispose",At),ce(L)}function ce(x){Ce(x),kt.remove(x)}function Ce(x){let L=kt.get(x).programs;if(L!==void 0){if(L.forEach(function(V){ht.releaseProgram(V)}),x.isShaderMaterial)ht.releaseShaderCache(x)}}this.renderBufferDirect=function(x,L,V,z,H,bt){if(L===null)L=Ue;let Pt=H.isMesh&&H.matrixWorld.determinantAffine()<0,_t=Dr(x,L,V,z,H);at.setMaterial(z,Pt);let A=V.index,C=1;if(z.wireframe===!0){if(A=_.getWireframeAttribute(V),A===void 0)return;C=2}let F=V.drawRange,K=V.attributes.position,O=F.start*C,ot=(F.start+F.count)*C;if(bt!==null)O=Math.max(O,bt.start*C),ot=Math.min(ot,(bt.start+bt.count)*C);if(A!==null)O=Math.max(O,0),ot=Math.min(ot,A.count);else if(K!==void 0&&K!==null)O=Math.max(O,0),ot=Math.min(ot,K.count);let xt=ot-O;if(xt<0||xt===1/0)return;ft.setup(H,z,_t,V,A);let Tt,Ct=Q;if(A!==null)Tt=b.get(A),Ct=Z,Ct.setIndex(Tt);if(H.isMesh)if(z.wireframe===!0)at.setLineWidth(z.wireframeLinewidth*Je()),Ct.setMode(U.LINES);else Ct.setMode(U.TRIANGLES);else if(H.isLine){let Ut=z.linewidth;if(Ut===void 0)Ut=1;if(at.setLineWidth(Ut*Je()),H.isLineSegments)Ct.setMode(U.LINES);else if(H.isLineLoop)Ct.setMode(U.LINE_LOOP);else Ct.setMode(U.LINE_STRIP)}else if(H.isPoints)Ct.setMode(U.POINTS);else if(H.isSprite)Ct.setMode(U.TRIANGLES);if(H.isBatchedMesh)if(!de.get("WEBGL_multi_draw")){let{_multiDrawStarts:Ut,_multiDrawCounts:lt,_multiDrawCount:$t}=H,Lt=A?b.get(A).bytesPerElement:1,se=kt.get(z).currentProgram.getUniforms();for(let wt=0;wt<$t;wt++)se.setValue(U,"_gl_DrawID",wt),Ct.render(Ut[wt]/Lt,lt[wt])}else Ct.renderMultiDraw(H._multiDrawStarts,H._multiDrawCounts,H._multiDrawCount);else if(H.isInstancedMesh)Ct.renderInstances(O,xt,H.count);else if(V.isInstancedBufferGeometry){let Ut=V._maxInstanceCount!==void 0?V._maxInstanceCount:1/0,lt=Math.min(V.instanceCount,Ut);Ct.renderInstances(O,xt,lt)}else Ct.render(O,xt)};function dn(x,L,V,z){if(N!==null&&x.isNodeMaterial)N.setObject(z,x);if(ge===!0)Y.setState(x,V,!1);if(x.transparent===!0&&x.side===rn&&x.forceSinglePass===!1)x.side=Qe,x.needsUpdate=!0,fn(x,L,z),x.side=Bi,x.needsUpdate=!0,fn(x,L,z),x.side=rn;else fn(x,L,z)}this.compile=function(x,L,V=null){if(V===null)V=x;if(N!==null)N.renderStart(x,L,V);if(E=Mt.get(V),E.init(L),g.push(E),V.traverseVisible(function(H){if(H.isLight&&H.layers.test(L.layers)){if(E.pushLight(H),H.castShadow)E.pushShadow(H)}}),x!==V)x.traverseVisible(function(H){if(H.isLight&&H.layers.test(L.layers)){if(E.pushLight(H),H.castShadow)E.pushShadow(H)}});if(E.setupLights(),N!==null)N.updateLights(E.state.lightsArray);if(Re=this.localClippingEnabled,ge=Y.init(this.clippingPlanes,Re),ge===!0)Y.setGlobalState(this.clippingPlanes,L);if(N!==null)et.render(E.state.shadowsArray,V,L);let z=new Set;if(x.traverse(function(H){if(!(H.isMesh||H.isPoints||H.isLine||H.isSprite))return;let bt=H.material;if(bt)if(Array.isArray(bt))for(let Pt=0;Pt<bt.length;Pt++){let _t=bt[Pt];dn(_t,V,L,H),z.add(_t)}else dn(bt,V,L,H),z.add(bt)}),E=g.pop(),N!==null)N.renderEnd();return z},this.compileAsync=function(x,L,V=null){let z=this.compile(x,L,V);return new Promise((H)=>{function bt(){if(z.forEach(function(Pt){let A=kt.get(Pt).currentProgram;if(A===void 0||A.isReady())z.delete(Pt)}),z.size===0){H(x);return}setTimeout(bt,10)}if(de.get("KHR_parallel_shader_compile")!==null)bt();else setTimeout(bt,10)})};let Yi=null;function we(x){if(Yi)Yi(x)}function jn(){Tn.stop()}function Ss(){Tn.start()}let Tn=new $c;if(Tn.setAnimationLoop(we),typeof self<"u")Tn.setContext(self);this.setAnimationLoop=function(x){Yi=x,Ft.setAnimationLoop(x),x===null?Tn.stop():Tn.start()},Ft.addEventListener("sessionstart",jn),Ft.addEventListener("sessionend",Ss),this.render=function(x,L){if(L!==void 0&&L.isCamera!==!0){Yt("WebGLRenderer.render: camera is not an instance of THREE.Camera.");return}if(q===!0)return;if(N!==null)N.renderStart(x,L);let V=Ft.enabled===!0&&Ft.isPresenting===!0,z=y!==null&&(G===null||V)&&y.begin(D,G);if(x.matrixWorldAutoUpdate===!0)x.updateMatrixWorld();if(L.parent===null&&L.matrixWorldAutoUpdate===!0)L.updateMatrixWorld();if(Ft.enabled===!0&&Ft.isPresenting===!0&&(y===null||y.isCompositing()===!1)){if(Ft.cameraAutoUpdate===!0)Ft.updateCamera(L);L=Ft.getCamera()}if(x.isScene===!0)x.onBeforeRender(D,x,L,G);if(E=Mt.get(x,g.length),E.init(L),E.state.textureUnits=Xt.getTextureUnits(),g.push(E),Le.multiplyMatrices(L.projectionMatrix,L.matrixWorldInverse),L.isArrayCamera)Me.setFromArrayCamera(L);else ie.setFromProjectionMatrix(Le,ro,L.reversedDepth);if(Re=this.localClippingEnabled,ge=Y.init(this.clippingPlanes,Re),S=mt.get(x,P.length),S.init(),P.push(S),this.info.render.frame++,Ft.enabled===!0&&Ft.isPresenting===!0){let Pt=D.xr.getDepthSensingMesh();if(Pt!==null)vi(Pt,L,-1/0,D.sortObjects)}if(vi(x,L,0,D.sortObjects),S.finish(),N!==null)N.updateLights(E.state.lightsArray);if(D.sortObjects===!0)S.sort(Dt,ue);if(Fe=Ft.enabled===!1||Ft.isPresenting===!1||Ft.hasDepthSensing()===!1,Fe)dt.addToRenderList(S,x);if(this.info.autoReset===!0)this.info.reset();if(ge===!0)Y.beginShadows();let H=E.state.shadowsArray;if(et.render(H,x,L),ge===!0)Y.endShadows();if((z&&y.hasRenderPass())===!1){let Pt=S.opaque,_t=S.transmissive;if(E.setupLights(),L.isArrayCamera){let A=L.cameras;if(_t.length>0)for(let C=0,F=A.length;C<F;C++){let K=A[C];ke(Pt,_t,x,K)}if(Fe)dt.render(x);for(let C=0,F=A.length;C<F;C++){let K=A[C];ze(S,x,K,K.viewport)}}else{if(_t.length>0)ke(Pt,_t,x,L);if(Fe)dt.render(x);ze(S,x,L)}}if(G!==null&&tt===0)Xt.updateMultisampleRenderTarget(G),Xt.updateRenderTargetMipmap(G);if(z)y.end(D);if(x.isScene===!0)x.onAfterRender(D,x,L);if(ft.resetDefaultState(),pt=-1,J=null,g.pop(),g.length>0){if(E=g[g.length-1],Xt.setTextureUnits(E.state.textureUnits),ge===!0)Y.setGlobalState(D.clippingPlanes,E.state.camera)}else E=null;if(P.pop(),P.length>0)S=P[P.length-1];else S=null;if(N!==null)N.renderEnd()};function vi(x,L,V,z){if(x.visible===!1)return;if(x.layers.test(L.layers)){if(x.isGroup)V=x.renderOrder;else if(x.isLOD){if(x.autoUpdate===!0)x.update(L)}else if(x.isLight){if(E.pushLight(x),x.castShadow)E.pushShadow(x)}else if(x.isSprite){let Pt=L.isArrayCamera?Me:ie;if(!x.frustumCulled||x.intersectsFrustum(Pt)){if(z)be.setFromMatrixPosition(x.matrixWorld).applyMatrix4(Le);let _t=B.update(x),A=x.material;if(A.visible)S.push(x,_t,A,V,be.z,null,L)}}else if(x.isMesh||x.isLine||x.isPoints){let Pt=L.isArrayCamera?Me:ie;if(!x.frustumCulled||x.intersectsFrustum(Pt)){let _t=B.update(x),A=x.material;if(z){if(x.boundingSphere!==void 0){if(x.boundingSphere===null)x.computeBoundingSphere();be.copy(x.boundingSphere.center)}else{if(_t.boundingSphere===null)_t.computeBoundingSphere();be.copy(_t.boundingSphere.center)}be.applyMatrix4(x.matrixWorld).applyMatrix4(Le)}if(Array.isArray(A)){let C=_t.groups;for(let F=0,K=C.length;F<K;F++){let O=C[F],ot=A[O.materialIndex];if(ot&&ot.visible)S.push(x,_t,ot,V,be.z,O,L)}}else if(A.visible)S.push(x,_t,A,V,be.z,null,L)}}}let bt=x.children;for(let Pt=0,_t=bt.length;Pt<_t;Pt++)vi(bt[Pt],L,V,z)}function ze(x,L,V,z){let{opaque:H,transmissive:bt,transparent:Pt}=x;if(E.setupLightsView(V),ge===!0)Y.setGlobalState(D.clippingPlanes,V);if(z)at.viewport(st.copy(z));if(H.length>0)je(H,L,V);if(bt.length>0)je(bt,L,V);if(Pt.length>0)je(Pt,L,V);at.buffers.depth.setTest(!0),at.buffers.depth.setMask(!0),at.buffers.color.setMask(!0),at.setPolygonOffset(!1)}function ke(x,L,V,z){if((V.isScene===!0?V.overrideMaterial:null)!==null)return;if(E.state.transmissionRenderTarget[z.id]===void 0){let O=de.has("EXT_color_buffer_half_float")||de.has("EXT_color_buffer_float");E.state.transmissionRenderTarget[z.id]=new cn(1,1,{generateMipmaps:!0,type:O?bn:xn,minFilter:Yn,samples:Math.max(4,xe.samples),stencilBuffer:s,resolveDepthBuffer:!1,resolveStencilBuffer:!1,storeMultisampledDepthBuffer:!1,storeMultisampledStencilBuffer:!1,colorSpace:ae.workingColorSpace})}let bt=E.state.transmissionRenderTarget[z.id],Pt=z.viewport||st;bt.setSize(Pt.z*D.transmissionResolutionScale,Pt.w*D.transmissionResolutionScale);let _t=D.getRenderTarget(),A=D.getActiveCubeFace(),C=D.getActiveMipmapLevel();if(D.setRenderTarget(bt),D.getClearColor(nt),ct=D.getClearAlpha(),ct<1)D.setClearColor(16777215,0.5);if(D.clear(),Fe)dt.render(V);let F=D.toneMapping;D.toneMapping=Sn;let K=z.viewport;if(z.viewport!==void 0)z.viewport=void 0;if(E.setupLightsView(z),ge===!0)Y.setGlobalState(D.clippingPlanes,z);if(je(x,V,z),Xt.updateMultisampleRenderTarget(bt),Xt.updateRenderTargetMipmap(bt),de.has("WEBGL_multisampled_render_to_texture")===!1){let O=!1;for(let ot=0,xt=L.length;ot<xt;ot++){let Tt=L[ot],{object:Ct,geometry:Ut,material:lt,group:$t}=Tt;if(lt.side===rn&&Ct.layers.test(z.layers)){let Lt=lt.side;lt.side=Qe,lt.needsUpdate=!0,ti(Ct,V,z,Ut,lt,$t),lt.side=Lt,lt.needsUpdate=!0,O=!0}}if(O===!0)Xt.updateMultisampleRenderTarget(bt),Xt.updateRenderTargetMipmap(bt)}if(D.setRenderTarget(_t,A,C),D.setClearColor(nt,ct),K!==void 0)z.viewport=K;D.toneMapping=F}function je(x,L,V){let z=L.isScene===!0?L.overrideMaterial:null;for(let H=0,bt=x.length;H<bt;H++){let Pt=x[H],{object:_t,geometry:A,group:C}=Pt,F=Pt.material;if(F.allowOverride===!0&&z!==null)F=z;if(_t.layers.test(V.layers))ti(_t,L,V,A,F,C)}}function ti(x,L,V,z,H,bt){if(N!==null&&H.isNodeMaterial)N.setObject(x,H);if(x.onBeforeRender(D,L,V,z,H,bt),x.modelViewMatrix.multiplyMatrices(V.matrixWorldInverse,x.matrixWorld),x.normalMatrix.getNormalMatrix(x.modelViewMatrix),H.onBeforeRender(D,L,V,z,x,bt),H.transparent===!0&&H.side===rn&&H.forceSinglePass===!1)H.side=Qe,H.needsUpdate=!0,D.renderBufferDirect(V,L,z,H,x,bt),H.side=Bi,H.needsUpdate=!0,D.renderBufferDirect(V,L,z,H,x,bt),H.side=rn;else D.renderBufferDirect(V,L,z,H,x,bt);x.onAfterRender(D,L,V,z,H,bt)}function En(x){let L=0;for(let V of x)if(V.texture!==null)L++;return L}function fn(x,L,V){if(L.isScene!==!0)L=Ue;let z=kt.get(x),H=E.state.lights,bt=E.state.shadowsArray,Pt=H.state.version,_t=ht.getParameters(x,H.state,bt,L,V,E.state.lightProbeGridArray),A=ht.getProgramCacheKey(_t),C=z.programs;z.environment=x.isMeshStandardMaterial||x.isMeshLambertMaterial||x.isMeshPhongMaterial?L.environment:null,z.fog=L.fog;let F=x.isMeshStandardMaterial||x.isMeshLambertMaterial&&!x.envMap||x.isMeshPhongMaterial&&!x.envMap;if(z.envMap=Oe.get(x.envMap||z.environment,F),z.envMapRotation=z.environment!==null&&x.envMap===null?L.environmentRotation:x.envMapRotation,C===void 0)x.addEventListener("dispose",At),C=new Map,z.programs=C,z.programVariants=new Map;let K=C.get(A);if(K!==void 0){if(z.currentProgram===K&&z.lightsStateVersion===Pt)return bs(x,_t),K}else{if(_t.uniforms=ht.getUniforms(x),N!==null&&x.isNodeMaterial)N.build(x,V,_t);x.onBeforeCompile(_t,D),K=ht.acquireProgram(_t,A),C.set(A,K),z.uniforms=_t.uniforms}let O=z.uniforms;if(!x.isShaderMaterial&&!x.isRawShaderMaterial||x.clipping===!0)O.clippingPlanes=Y.uniform;if(bs(x,_t),z.needsLights=Ur(x),z.lightsStateVersion=Pt,z.needsLights)O.ambientLightColor.value=H.state.ambient,O.lightProbe.value=H.state.probe,O.sunLights.value=H.state.sun,O.sunLightShadows.value=H.state.sunShadow,O.directionalLights.value=H.state.directional,O.directionalLightShadows.value=H.state.directionalShadow,O.spotLights.value=H.state.spot,O.spotLightShadows.value=H.state.spotShadow,O.rectAreaLights.value=H.state.rectArea,O.ltc_1.value=H.state.rectAreaLTC1,O.ltc_2.value=H.state.rectAreaLTC2,O.pointLights.value=H.state.point,O.pointLightShadows.value=H.state.pointShadow,O.hemisphereLights.value=H.state.hemi,O.sunShadowMatrix.value=H.state.sunShadowMatrix,O.sunShadowCascade.value=H.state.sunShadowCascade,O.directionalShadowMatrix.value=H.state.directionalShadowMatrix,O.spotLightMatrix.value=H.state.spotLightMatrix,O.spotLightMap.value=H.state.spotLightMap,O.pointShadowMatrix.value=H.state.pointShadowMatrix;return z.numLightProbeGrids=En(E.state.lightProbeGridArray),z.currentProgram=K,z.uniformsList=null,K}function Zi(x,L,V,z,H){let bt=kt.get(x),Pt=bt.currentProgram.id+","+z+","+H,_t=bt.programVariants.get(Pt);if(_t===void 0)_t=fn(x,L,V),bt.programVariants.set(Pt,_t);else bt.currentProgram=_t,bt.uniformsList=null,bt.outputColorSpace=z,bt.toneMapping=H;return _t}function ei(x){if(x.uniformsList===null){let L=x.currentProgram.getUniforms();x.uniformsList=_s.seqWithValue(L.seq,x.uniforms)}return x.uniformsList}function bs(x,L){let V=kt.get(x);V.outputColorSpace=L.outputColorSpace,V.batching=L.batching,V.batchingColor=L.batchingColor,V.instancing=L.instancing,V.instancingColor=L.instancingColor,V.instancingMorph=L.instancingMorph,V.skinning=L.skinning,V.morphTargets=L.morphTargets,V.morphNormals=L.morphNormals,V.morphColors=L.morphColors,V.morphTargetsCount=L.morphTargetsCount,V.numClippingPlanes=L.numClippingPlanes,V.numIntersection=L.numClipIntersection,V.vertexAlphas=L.vertexAlphas,V.vertexTangents=L.vertexTangents,V.toneMapping=L.toneMapping}function Dr(x,L,V,z,H){if(L.isScene!==!0)L=Ue;Xt.resetTextureUnits();let bt=L.fog,Pt=z.isMeshStandardMaterial||z.isMeshLambertMaterial||z.isMeshPhongMaterial?L.environment:null,_t=G===null?D.outputColorSpace:G.isXRRenderTarget===!0?G.texture.colorSpace:ae.workingColorSpace,A=z.isMeshStandardMaterial||z.isMeshLambertMaterial&&!z.envMap||z.isMeshPhongMaterial&&!z.envMap,C=Oe.get(z.envMap||Pt,A),F=z.vertexColors===!0&&!!V.attributes.color&&V.attributes.color.itemSize===4,K=!!V.attributes.tangent&&(!!z.normalMap||z.anisotropy>0),O=!!V.morphAttributes.position,ot=!!V.morphAttributes.normal,xt=!!V.morphAttributes.color,Tt=Sn;if(z.toneMapped){if(G===null||G.isXRRenderTarget===!0)Tt=D.toneMapping}let Ct=V.morphAttributes.position||V.morphAttributes.normal||V.morphAttributes.color,Ut=Ct!==void 0?Ct.length:0,lt=kt.get(z),$t=E.state.lights;if(ge===!0){if(Re===!0||x!==J){let te=x===J&&z.id===pt;Y.setState(z,x,te)}}let Lt=!1,se=!1;if(z.version===lt.__version){if(lt.needsLights&&lt.lightsStateVersion!==$t.state.version)Lt=!0;else if(H.isBatchedMesh&&lt.batching===!1)Lt=!0;else if(!H.isBatchedMesh&&lt.batching===!0)Lt=!0;else if(H.isBatchedMesh&&lt.batchingColor===!0&&H._colorsTexture===null)Lt=!0;else if(H.isBatchedMesh&&lt.batchingColor===!1&&H._colorsTexture!==null)Lt=!0;else if(H.isInstancedMesh&&lt.instancing===!1)Lt=!0;else if(!H.isInstancedMesh&&lt.instancing===!0)Lt=!0;else if(H.isSkinnedMesh&&lt.skinning===!1)Lt=!0;else if(!H.isSkinnedMesh&&lt.skinning===!0)Lt=!0;else if(H.isInstancedMesh&&lt.instancingColor===!0&&H.instanceColor===null)Lt=!0;else if(H.isInstancedMesh&&lt.instancingColor===!1&&H.instanceColor!==null)Lt=!0;else if(H.isInstancedMesh&&lt.instancingMorph===!0&&H.morphTexture===null)Lt=!0;else if(H.isInstancedMesh&&lt.instancingMorph===!1&&H.morphTexture!==null)Lt=!0;else if(lt.envMap!==C)Lt=!0;else if(z.fog===!0&&lt.fog!==bt)Lt=!0;else if(lt.numClippingPlanes!==void 0&&(lt.numClippingPlanes!==Y.numPlanes||lt.numIntersection!==Y.numIntersection))Lt=!0;else if(lt.vertexAlphas!==F)Lt=!0;else if(lt.vertexTangents!==K)Lt=!0;else if(lt.morphTargets!==O)Lt=!0;else if(lt.morphNormals!==ot)Lt=!0;else if(lt.morphColors!==xt)Lt=!0;else if(lt.morphTargetsCount!==Ut)Lt=!0;else if(lt.numLightProbeGrids!==En(E.state.lightProbeGridArray))Lt=!0;else if(lt.outputColorSpace!==_t||lt.toneMapping!==Tt)se=!0}else Lt=!0,lt.__version=z.version;let wt=lt.currentProgram;if(Lt===!0||se===!0){if(wt=se===!0?Zi(z,L,H,_t,Tt):fn(z,L,H),N&&z.isNodeMaterial)N.onUpdateProgram(z,wt,lt)}let gt=!1,Ht=!1,vt=!1,Nt=wt.getUniforms(),Kt=lt.uniforms;if(at.useProgram(wt.program))gt=!0,Ht=!0,vt=!0;if(z.id!==pt)pt=z.id,Ht=!0;if(gt||J!==x){if(at.buffers.depth.getReversed()&&x.reversedDepth!==!0)x._reversedDepth=!0,x.updateProjectionMatrix();Nt.setValue(U,"projectionMatrix",x.projectionMatrix),Nt.setValue(U,"viewMatrix",x.matrixWorldInverse);let Qt=Nt.map.cameraPosition;if(Qt!==void 0)Qt.setValue(U,_e.setFromMatrixPosition(x.matrixWorld));if(xe.logarithmicDepthBuffer)Nt.setValue(U,"logDepthBufFC",2/(Math.log(x.far+1)/Math.LN2));if(z.isMeshPhongMaterial||z.isMeshToonMaterial||z.isMeshLambertMaterial||z.isMeshBasicMaterial||z.isMeshStandardMaterial||z.isShaderMaterial)Nt.setValue(U,"isOrthographic",x.isOrthographicCamera===!0);if(J!==x)J=x,Ht=!0,vt=!0}if(lt.needsLights){if($t.state.sunShadowMap.length>0)Nt.setValue(U,"sunShadowMap",$t.state.sunShadowMap,Xt);if($t.state.directionalShadowMap.length>0)Nt.setValue(U,"directionalShadowMap",$t.state.directionalShadowMap,Xt);if($t.state.spotShadowMap.length>0)Nt.setValue(U,"spotShadowMap",$t.state.spotShadowMap,Xt);if($t.state.pointShadowMap.length>0)Nt.setValue(U,"pointShadowMap",$t.state.pointShadowMap,Xt)}if(H.isSkinnedMesh){Nt.setOptional(U,H,"bindMatrix"),Nt.setOptional(U,H,"bindMatrixInverse");let te=H.skeleton;if(te){if(te.boneTexture===null)te.computeBoneTexture();Nt.setValue(U,"boneTexture",te.boneTexture,Xt)}}if(H.isBatchedMesh){if(Nt.setOptional(U,H,"batchingTexture"),Nt.setValue(U,"batchingTexture",H._matricesTexture,Xt),Nt.setOptional(U,H,"batchingIdTexture"),Nt.setValue(U,"batchingIdTexture",H._indirectTexture,Xt),Nt.setOptional(U,H,"batchingColorTexture"),H._colorsTexture!==null)Nt.setValue(U,"batchingColorTexture",H._colorsTexture,Xt)}let qt=V.morphAttributes;if(qt.position!==void 0||qt.normal!==void 0||qt.color!==void 0)I.update(H,V,wt);if(Ht||lt.receiveShadow!==H.receiveShadow)lt.receiveShadow=H.receiveShadow,Nt.setValue(U,"receiveShadow",H.receiveShadow);if((z.isMeshStandardMaterial||z.isMeshLambertMaterial||z.isMeshPhongMaterial)&&z.envMap===null&&L.environment!==null)Kt.envMapIntensity.value=L.environmentIntensity;if(Kt.dfgLUT!==void 0)Kt.dfgLUT.value=Ym();if(Ht){if(Nt.setValue(U,"toneMappingExposure",D.toneMappingExposure),lt.needsLights)ws(Kt,vt);if(bt&&z.fog===!0)it.refreshFogUniforms(Kt,bt);if(it.refreshMaterialUniforms(Kt,z,zt,Gt,E.state.transmissionRenderTarget[x.id]),lt.needsLights&&lt.numLightProbeGrids>0){let te=T,Qt=0;for(let Ie of E.state.lightProbeGridArray){if(Ie.texture===null)continue;te.textures[Qt]=Ie.texture,te.min[Qt]=Ie.boundingBox.min,te.max[Qt]=Ie.boundingBox.max,te.resolution[Qt]=Ie.resolution,Qt++}for(let Ie in te)te[Ie].length=Qt;Kt.probesSH.value=te.textures,Kt.probesMin.value=te.min,Kt.probesMax.value=te.max,Kt.probesResolution.value=te.resolution}_s.upload(U,ei(lt),Kt,Xt)}if(z.isShaderMaterial&&z.uniformsNeedUpdate===!0)_s.upload(U,ei(lt),Kt,Xt),z.uniformsNeedUpdate=!1;if(z.isSpriteMaterial)Nt.setValue(U,"center",H.center);if(Nt.setValue(U,"modelViewMatrix",H.modelViewMatrix),Nt.setValue(U,"normalMatrix",H.normalMatrix),Nt.setValue(U,"modelMatrix",H.matrixWorld),z.uniformsGroups!==void 0){let te=z.uniformsGroups;for(let Qt=0,Ie=te.length;Qt<Ie;Qt++){let Te=te[Qt];St.update(Te,wt),St.bind(Te,wt)}}return wt}function ws(x,L){x.ambientLightColor.needsUpdate=L,x.lightProbe.needsUpdate=L,x.sunLights.needsUpdate=L,x.sunLightShadows.needsUpdate=L,x.directionalLights.needsUpdate=L,x.directionalLightShadows.needsUpdate=L,x.pointLights.needsUpdate=L,x.pointLightShadows.needsUpdate=L,x.spotLights.needsUpdate=L,x.spotLightShadows.needsUpdate=L,x.rectAreaLights.needsUpdate=L,x.hemisphereLights.needsUpdate=L}function Ur(x){return x.isMeshLambertMaterial||x.isMeshToonMaterial||x.isMeshPhongMaterial||x.isMeshStandardMaterial||x.isShadowMaterial||x.isShaderMaterial&&x.lights===!0}this.getActiveCubeFace=function(){return j},this.getActiveMipmapLevel=function(){return tt},this.getRenderTarget=function(){return G},this.setRenderTargetTextures=function(x,L,V){let z=kt.get(x);if(z.__autoAllocateDepthBuffer=x.resolveDepthBuffer===!1,z.__autoAllocateDepthBuffer===!1)z.__useRenderToTexture=!1;kt.get(x.texture).__webglTexture=L,kt.get(x.depthTexture).__webglTexture=z.__autoAllocateDepthBuffer?void 0:V,z.__hasExternalTextures=!0},this.setRenderTargetFramebuffer=function(x,L){let V=kt.get(x);V.__webglFramebuffer=L,V.__useDefaultFramebuffer=L===void 0},this.setRenderTarget=function(x,L=0,V=0){G=x,j=L,tt=V;let z=null,H=!1,bt=!1;if(x){let _t=kt.get(x);if(_t.__useDefaultFramebuffer!==void 0){at.bindFramebuffer(U.FRAMEBUFFER,_t.__webglFramebuffer),st.copy(x.viewport),k.copy(x.scissor),It=x.scissorTest,at.viewport(st),at.scissor(k),at.setScissorTest(It),pt=-1;return}else if(_t.__webglFramebuffer===void 0)Xt.setupRenderTarget(x);else if(_t.__hasExternalTextures)Xt.rebindTextures(x,kt.get(x.texture).__webglTexture,kt.get(x.depthTexture).__webglTexture);else if(x.depthBuffer){let F=x.depthTexture;if(_t.__boundDepthTexture!==F){if(F!==null&&kt.has(F)&&(x.width!==F.image.width||x.height!==F.image.height))throw Error("THREE.WebGLRenderer: Attached DepthTexture is initialized to the incorrect size.");Xt.setupDepthRenderbuffer(x)}}let A=x.texture;if(A.isData3DTexture||A.isDataArrayTexture||A.isCompressedArrayTexture)bt=!0;let C=kt.get(x).__webglFramebuffer;if(x.isWebGLCubeRenderTarget){if(Array.isArray(C[L]))z=C[L][V];else z=C[L];H=!0}else if(x.samples>0&&Xt.useMultisampledRTT(x)===!1)z=kt.get(x).__webglMultisampledFramebuffer;else if(Array.isArray(C))z=C[V];else z=C;st.copy(x.viewport),k.copy(x.scissor),It=x.scissorTest}else st.copy(ee).multiplyScalar(zt).floor(),k.copy(Jt).multiplyScalar(zt).floor(),It=ve;if(V!==0)z=X;if(at.bindFramebuffer(U.FRAMEBUFFER,z))at.drawBuffers(x,z);if(at.viewport(st),at.scissor(k),at.setScissorTest(It),H){let _t=kt.get(x.texture);U.framebufferTexture2D(U.FRAMEBUFFER,U.COLOR_ATTACHMENT0,U.TEXTURE_CUBE_MAP_POSITIVE_X+L,_t.__webglTexture,V)}else if(bt){let _t=L;for(let A=0;A<x.textures.length;A++){let C=kt.get(x.textures[A]);U.framebufferTextureLayer(U.FRAMEBUFFER,U.COLOR_ATTACHMENT0+A,C.__webglTexture,V,_t)}}else if(x!==null&&V!==0){let _t=kt.get(x.texture);U.framebufferTexture2D(U.FRAMEBUFFER,U.COLOR_ATTACHMENT0,U.TEXTURE_2D,_t.__webglTexture,V)}pt=-1};function As(x){let L=kt.get(x);if(L.__readFormat!==x.format||L.__readType!==x.type)L.__readFormat=x.format,L.__readType=x.type,L.__formatReadable=xe.textureFormatReadable(x.format),L.__typeReadable=xe.textureTypeReadable(x.type);return L}if(this.readRenderTargetPixels=function(x,L,V,z,H,bt,Pt,_t=0){if(!(x&&x.isWebGLRenderTarget)){Yt("WebGLRenderer.readRenderTargetPixels: renderTarget is not THREE.WebGLRenderTarget.");return}let A=kt.get(x).__webglFramebuffer;if(x.isWebGLCubeRenderTarget&&Pt!==void 0){if(A=A[Pt],Array.isArray(A))A=A[0]}if(A){at.bindFramebuffer(U.FRAMEBUFFER,A);try{let C=x.textures[_t],{format:F,type:K}=C;if(x.textures.length>1)U.readBuffer(U.COLOR_ATTACHMENT0+_t);let O=As(C);if(O.__formatReadable===!1){Yt("WebGLRenderer.readRenderTargetPixels: renderTarget is not in RGBA or implementation defined format.");return}if(O.__typeReadable===!1){Yt("WebGLRenderer.readRenderTargetPixels: renderTarget is not in UnsignedByteType or implementation defined type.");return}if(L>=0&&L<=x.width-z&&(V>=0&&V<=x.height-H))U.readPixels(L,V,z,H,ut.convert(F),ut.convert(K),bt)}finally{let C=G!==null?kt.get(G).__webglFramebuffer:null;at.bindFramebuffer(U.FRAMEBUFFER,C)}}},this.readRenderTargetPixelsAsync=async function(x,L,V,z,H,bt,Pt,_t=0){if(!(x&&x.isWebGLRenderTarget))throw Error("THREE.WebGLRenderer.readRenderTargetPixels: renderTarget is not THREE.WebGLRenderTarget.");let A=kt.get(x).__webglFramebuffer;if(x.isWebGLCubeRenderTarget&&Pt!==void 0){if(A=A[Pt],Array.isArray(A))A=A[0]}if(A)if(L>=0&&L<=x.width-z&&(V>=0&&V<=x.height-H)){at.bindFramebuffer(U.FRAMEBUFFER,A);let C=x.textures[_t],{format:F,type:K}=C;if(x.textures.length>1)U.readBuffer(U.COLOR_ATTACHMENT0+_t);let O=As(C);if(O.__formatReadable===!1)throw Error("THREE.WebGLRenderer.readRenderTargetPixelsAsync: renderTarget is not in RGBA or implementation defined format.");if(O.__typeReadable===!1)throw Error("THREE.WebGLRenderer.readRenderTargetPixelsAsync: renderTarget is not in UnsignedByteType or implementation defined type.");let ot=U.createBuffer();U.bindBuffer(U.PIXEL_PACK_BUFFER,ot),U.bufferData(U.PIXEL_PACK_BUFFER,bt.byteLength,U.STREAM_READ),U.readPixels(L,V,z,H,ut.convert(F),ut.convert(K),0),U.bindBuffer(U.PIXEL_PACK_BUFFER,null);let xt=G!==null?kt.get(G).__webglFramebuffer:null;at.bindFramebuffer(U.FRAMEBUFFER,xt);let Tt=U.fenceSync(U.SYNC_GPU_COMMANDS_COMPLETE,0);return U.flush(),await yc(U,Tt,4),U.bindBuffer(U.PIXEL_PACK_BUFFER,ot),U.getBufferSubData(U.PIXEL_PACK_BUFFER,0,bt),U.bindBuffer(U.PIXEL_PACK_BUFFER,null),U.deleteBuffer(ot),U.deleteSync(Tt),bt}else throw Error("THREE.WebGLRenderer.readRenderTargetPixelsAsync: requested read bounds are out of range.")},this.copyFramebufferToTexture=function(x,L=null,V=0){let z=Math.pow(2,-V),H=Math.floor(x.image.width*z),bt=Math.floor(x.image.height*z),Pt=L!==null?L.x:0,_t=L!==null?L.y:0;Xt.setTexture2D(x,0),U.copyTexSubImage2D(U.TEXTURE_2D,V,0,0,Pt,_t,H,bt),at.unbindTexture()},this.copyTextureToTexture=function(x,L,V=null,z=null,H=0,bt=0){let Pt,_t,A,C,F,K,O,ot,xt,Tt=x.isCompressedTexture?x.mipmaps[bt]:x.image;if(V!==null)Pt=V.max.x-V.min.x,_t=V.max.y-V.min.y,A=V.isBox3?V.max.z-V.min.z:1,C=V.min.x,F=V.min.y,K=V.isBox3?V.min.z:0;else{let Nt=Math.pow(2,-H);if(Pt=Math.floor(Tt.width*Nt),_t=Math.floor(Tt.height*Nt),x.isDataArrayTexture)A=Tt.depth;else if(x.isData3DTexture)A=Math.floor(Tt.depth*Nt);else A=1;C=0,F=0,K=0}if(z!==null)O=z.x,ot=z.y,xt=z.z;else O=0,ot=0,xt=0;let Ct=ut.convert(L.format),Ut=ut.convert(L.type),lt;if(L.isData3DTexture)Xt.setTexture3D(L,0),lt=U.TEXTURE_3D;else if(L.isDataArrayTexture||L.isCompressedArrayTexture)Xt.setTexture2DArray(L,0),lt=U.TEXTURE_2D_ARRAY;else Xt.setTexture2D(L,0),lt=U.TEXTURE_2D;at.activeTexture(U.TEXTURE0),at.pixelStorei(U.UNPACK_FLIP_Y_WEBGL,L.flipY),at.pixelStorei(U.UNPACK_PREMULTIPLY_ALPHA_WEBGL,L.premultiplyAlpha),at.pixelStorei(U.UNPACK_ALIGNMENT,L.unpackAlignment);let $t=at.getParameter(U.UNPACK_ROW_LENGTH),Lt=at.getParameter(U.UNPACK_IMAGE_HEIGHT),se=at.getParameter(U.UNPACK_SKIP_PIXELS),wt=at.getParameter(U.UNPACK_SKIP_ROWS),gt=at.getParameter(U.UNPACK_SKIP_IMAGES);at.pixelStorei(U.UNPACK_ROW_LENGTH,Tt.width),at.pixelStorei(U.UNPACK_IMAGE_HEIGHT,Tt.height),at.pixelStorei(U.UNPACK_SKIP_PIXELS,C),at.pixelStorei(U.UNPACK_SKIP_ROWS,F),at.pixelStorei(U.UNPACK_SKIP_IMAGES,K);let Ht=x.isDataArrayTexture||x.isData3DTexture,vt=L.isDataArrayTexture||L.isData3DTexture;if(x.isDepthTexture){let Nt=kt.get(x),Kt=kt.get(L),qt=kt.get(Nt.__renderTarget),te=kt.get(Kt.__renderTarget);at.bindFramebuffer(U.READ_FRAMEBUFFER,qt.__webglFramebuffer),at.bindFramebuffer(U.DRAW_FRAMEBUFFER,te.__webglFramebuffer);for(let Qt=0;Qt<A;Qt++){if(Ht)U.framebufferTextureLayer(U.READ_FRAMEBUFFER,U.COLOR_ATTACHMENT0,kt.get(x).__webglTexture,H,K+Qt),U.framebufferTextureLayer(U.DRAW_FRAMEBUFFER,U.COLOR_ATTACHMENT0,kt.get(L).__webglTexture,bt,xt+Qt);U.blitFramebuffer(C,F,C+Pt,F+_t,O,ot,O+Pt,ot+_t,U.DEPTH_BUFFER_BIT,U.NEAREST)}at.bindFramebuffer(U.READ_FRAMEBUFFER,null),at.bindFramebuffer(U.DRAW_FRAMEBUFFER,null)}else if(H!==0||x.isRenderTargetTexture||kt.has(x)){let Nt=kt.get(x),Kt=kt.get(L);at.bindFramebuffer(U.READ_FRAMEBUFFER,rt),at.bindFramebuffer(U.DRAW_FRAMEBUFFER,R);for(let qt=0;qt<A;qt++){if(Ht)U.framebufferTextureLayer(U.READ_FRAMEBUFFER,U.COLOR_ATTACHMENT0,Nt.__webglTexture,H,K+qt);else U.framebufferTexture2D(U.READ_FRAMEBUFFER,U.COLOR_ATTACHMENT0,U.TEXTURE_2D,Nt.__webglTexture,H);if(vt)U.framebufferTextureLayer(U.DRAW_FRAMEBUFFER,U.COLOR_ATTACHMENT0,Kt.__webglTexture,bt,xt+qt);else U.framebufferTexture2D(U.DRAW_FRAMEBUFFER,U.COLOR_ATTACHMENT0,U.TEXTURE_2D,Kt.__webglTexture,bt);if(H!==0)U.blitFramebuffer(C,F,C+Pt,F+_t,O,ot,O+Pt,ot+_t,U.COLOR_BUFFER_BIT,U.NEAREST);else if(vt)U.copyTexSubImage3D(lt,bt,O,ot,xt+qt,C,F,Pt,_t);else U.copyTexSubImage2D(lt,bt,O,ot,C,F,Pt,_t)}at.bindFramebuffer(U.READ_FRAMEBUFFER,null),at.bindFramebuffer(U.DRAW_FRAMEBUFFER,null)}else if(vt)if(x.isDataTexture||x.isData3DTexture)U.texSubImage3D(lt,bt,O,ot,xt,Pt,_t,A,Ct,Ut,Tt.data);else if(L.isCompressedArrayTexture)U.compressedTexSubImage3D(lt,bt,O,ot,xt,Pt,_t,A,Ct,Tt.data);else U.texSubImage3D(lt,bt,O,ot,xt,Pt,_t,A,Ct,Ut,Tt);else if(x.isDataTexture)U.texSubImage2D(U.TEXTURE_2D,bt,O,ot,Pt,_t,Ct,Ut,Tt.data);else if(x.isCompressedTexture)U.compressedTexSubImage2D(U.TEXTURE_2D,bt,O,ot,Tt.width,Tt.height,Ct,Tt.data);else U.texSubImage2D(U.TEXTURE_2D,bt,O,ot,Pt,_t,Ct,Ut,Tt);if(at.pixelStorei(U.UNPACK_ROW_LENGTH,$t),at.pixelStorei(U.UNPACK_IMAGE_HEIGHT,Lt),at.pixelStorei(U.UNPACK_SKIP_PIXELS,se),at.pixelStorei(U.UNPACK_SKIP_ROWS,wt),at.pixelStorei(U.UNPACK_SKIP_IMAGES,gt),bt===0&&L.generateMipmaps===!0&&L.mipmapsAutoUpdate===!0)U.generateMipmap(lt);at.unbindTexture()},this.initRenderTarget=function(x){if(kt.get(x).__webglFramebuffer===void 0)Xt.setupRenderTarget(x)},this.initTexture=function(x){if(x.isCubeTexture)Xt.setTextureCube(x,0);else if(x.isData3DTexture)Xt.setTexture3D(x,0);else if(x.isDataArrayTexture||x.isCompressedArrayTexture)Xt.setTexture2DArray(x,0);else Xt.setTexture2D(x,0);at.unbindTexture()},this.resetState=function(){j=0,tt=0,G=null,at.reset(),ft.reset()},typeof __THREE_DEVTOOLS__<"u")__THREE_DEVTOOLS__.dispatchEvent(new CustomEvent("observe",{detail:this}))}get coordinateSystem(){return ro}get outputColorSpace(){return this._outputColorSpace}set outputColorSpace(t){this._outputColorSpace=t;let e=this.getContext();e.drawingBufferColorSpace=ae._getDrawingBufferColorSpace(t),e.unpackColorSpace=ae._getUnpackColorSpace()}}var _i=86400000;var ph=(t)=>t.places.filter((e)=>e.vis),ko=(t)=>t.efforts.filter((e)=>e.vis);var mh=(t)=>t.length*0.56,vs=new Map;function gh(){let t=document.createElement("canvas").getContext("2d");if(!t)return;t.font='500 100px -apple-system, BlinkMacSystemFont, "Segoe UI", Inter, Roboto, system-ui, sans-serif',mh=(e)=>t.measureText(e).width/100*1.03,vs.clear()}function Xi(t){let e=vs.get(t);if(e===void 0){if(e=mh(t),vs.size>20000)vs.clear();vs.set(t,e)}return e}function _h(t,e,n){if(Xi(t)*e<=n)return t;let i=0,s=t.length;while(i<s){let r=i+s+1>>1;if(Xi(t.slice(0,r).trimEnd()+"…")*e<=n)i=r;else s=r-1}return i?t.slice(0,i).trimEnd()+"…":""}var Qn=330,zn=1.3,Rr=0.25,Zm=[[0,"today"],[1,"yesterday"],[3,"3 days"],[7,"a week"],[30,"a month"],[91,"3 months"]],Jm=(t)=>Math.min(120,Math.max(1.5,t*1.08));function xh(t){let e=Math.log1p(t/Rr),n=(s)=>Qn*(1-Math.min(1,Math.log1p(Math.max(0,s)/Rr)/e)),i=Zm.filter(([s])=>s<=t).map(([s,r])=>({days:s,label:r,y:n(s)}));return{span:t,y:n,levels:i}}function $m(t,e=1,n=1,i=0){let s=new Map,r=(u)=>{if(s.has(u.id))return s.get(u.id);s.set(u.id,1);let d=0;for(let v of u.kids)d+=r(v);return d=Math.max(1,d),s.set(u.id,d),d};r(t);let a=new Map,o=[],l=new Set([t.id]),c=(u,d,v,w)=>{let f=u.kids.reduce((M,T)=>M+s.get(T.id),0),m=d;for(let M of u.kids){if(l.has(M.id))continue;l.add(M.id);let T=(v-d)*s.get(M.id)/f,S=m+T/2;a.set(M.id,S),(o[w+1]??=[]).push({id:M.id,a:S}),c(M,m,m+T,w+1),m+=T}};c(t,i,i+Math.PI*2,0);let p=[0];for(let u=1;u<o.length;u++){let d=(o[u]??[]).map((f)=>f.a).sort((f,m)=>f-m),v=Math.PI*2;for(let f=0;f<d.length;f++){let m=f+1<d.length?d[f+1]:d[0]+Math.PI*2;if(d.length>1)v=Math.min(v,m-d[f])}let w=v>=Math.PI?e/2:e/(2*Math.sin(v/2));p[u]=Math.max(p[u-1]+n,w)}let h=new Map([[t.id,{x:0,z:0,depth:0}]]);for(let u=1;u<o.length;u++)for(let d of o[u]??[])h.set(d.id,{x:Math.cos(d.a)*p[u],z:Math.sin(d.a)*p[u],depth:u});return{pos:h,ext:p[p.length-1]??0}}function Km(t){return t.map((e,n)=>{let i=1/0;for(let s=0;s<t.length;s++){if(s===n)continue;let r=t[s];i=Math.min(i,Math.hypot(e.x-r.x,e.y-r.y)-e.r-r.r)}return e.r+(Number.isFinite(i)?Math.max(0,i)/2:e.r)})}var pe={Child:0,Root:1,Lone:2,Work:3,Need:4},Qm=7,jm=20,tg=(t)=>{let e=2166136261;for(let n=0;n<t.length;n++)e=Math.imul(e^t.charCodeAt(n),16777619);return(e>>>0)/4294967296},Cr=(t,e,n)=>({x:(e-t.size/2)*zn,z:(n-t.size/2)*zn}),vh=(t,e,n)=>({x:e/zn+t.size/2,y:n/zn+t.size/2});function Ho(t,e,n){let i={items:[],efforts:[],places:[],byId:new Map,eIdx:new Map,scale:xh(1.5),radius:1};if(!e)return i;i.radius=e.size/2*zn;let s=ph(t),r=0,a=[];for(let c of s)for(let p of ko(c)){let h=e.efforts.get(p.id);if(!h)continue;a.push({e:p,c:h});for(let u of p.threads)if(!u.synthetic)r=Math.max(r,(n-u.e)/_i)}let o=i.scale=xh(Jm(r)),l=Km(a.map((c)=>c.c));a.forEach(({e:c,c:p},h)=>{let u=Cr(e,p.x,p.y),d=c.threads.filter((E)=>!E.synthetic).length,v=$m(c.root,1,1.15,tg(c.id)*Math.PI*2),w=l[h]*zn*0.94,f=v.ext>0?Math.min(jm,w*0.82/v.ext):Qm,m={e:c,cx:u.x,cz:u.z,cy:0,top:0,ext:v.ext*f,rootIdx:-1,items:[],n:d,score:0},M=c.root.kids.length===0;for(let E of c.threads){let P=v.pos.get(E.id);if(!P)continue;let g=E.id===c.root.id,y=E.need?pe.Need:E.working?pe.Work:g?M?pe.Lone:pe.Root:pe.Child,D=Math.min(4.2,f*0.62),q=g?M?3.6:Math.min(5+1.6*Math.log2(d+1),Math.max(D*1.3,2.3*f-D)):D,N=i.items.length;if(i.byId.set(E.id,N),m.items.push(N),g)m.rootIdx=N;i.items.push({t:E,x:u.x+P.x*f,y:o.y((n-E.e)/_i),z:u.z+P.z*f,eff:i.efforts.length,parent:-1,root:g,size:q,kind:y})}let T=0;for(let E of m.items){let P=i.items[E].y;T+=P,m.top=Math.max(m.top,P)}m.cy=m.items.length?T/m.items.length:0;let S=1-Math.min(1,Math.log1p(Math.max(0,n-c.e)/_i/Rr)/Math.log1p(o.span/Rr));m.score=Math.log2(d+1)/6+0.8*S+(c.need?1:0)+(c.working?0.6:0)-(c.root.synthetic?0.5:0),i.eIdx.set(c.id,i.efforts.length),i.efforts.push(m)});for(let c of i.items){if(c.root)continue;let p=c.t,h=p.up?.id??p.parent,u=h?i.byId.get(h):void 0;if(u!==void 0&&i.items[u].eff===c.eff)c.parent=u}for(let c of s){let p=e.places.get(c.key);if(!p)continue;let h=Cr(e,p.x,p.y),u=0;for(let d of ko(c)){let v=i.eIdx.get(d.id);if(v!==void 0)u=Math.max(u,i.efforts[v].top)}i.places.push({p:c,cx:h.x,cz:h.z,r:p.r*zn,top:u})}return i}var eg=(t,e,n)=>t.x-t.w/2-n<e.x+e.w/2&&t.x+t.w/2+n>e.x-e.w/2&&t.y-t.h-n<e.y&&t.y+n>e.y-e.h,Go=0.6,ng=(t,e)=>(e.force?1:0)-(t.force?1:0)||e.prio+(e.shown?Go:0)-(t.prio+(t.shown?Go:0))||(t.key<e.key?-1:t.key>e.key?1:0),ys=[];function yh(t,e,n,i=5,s=0.6){Go=s,t.sort(ng),ys.length=0;let r=0;for(let o of t){if(o.on=!1,!o.force&&r>=e)continue;let l=o.x-o.w/2,c=o.y-o.h,p=l>=n.x&&c>=n.y&&l+o.w<=n.x+n.w&&o.y<=n.y+n.h;if(!o.force&&!p)continue;let h=!1;if(!o.force){for(let u of ys)if(eg(o,u,i)){h=!0;break}}if(h)continue;if(o.on=!0,ys.push(o),!o.force)r++}let a=ys.length;return ys.length=0,a}var wh=46,xi=Math.tan(wh/2*Math.PI/180),Ms=12,ig=20,sg=650,rg=0.022,Wo=9000,ag=180000,Vo=1.45,og=658965,lg=`
attribute vec3 p0; attribute vec3 color; attribute float size; attribute vec2 pxr; attribute float alpha; attribute float a0;
uniform float uMix, uScale, uDpr, uFogN, uFogF;
varying vec3 vC; varying float vA; varying float vPx;
void main() {
  vec4 mv = modelViewMatrix * vec4(mix(p0, position, uMix), 1.0);
  float d = max(1.0, -mv.z);
  float px = clamp(size * uScale / d, pxr.x * uDpr, pxr.y * uDpr);
  gl_PointSize = px + uDpr;
  vPx = gl_PointSize;
  gl_Position = projectionMatrix * mv;
  vC = color;
  vA = alpha * mix(a0, 1.0, uMix) * (0.32 + 0.68 * (1.0 - smoothstep(uFogN, uFogF, d)));
}`,cg=`
uniform float uDpr;
varying vec3 vC; varying float vA; varying float vPx;
void main() {
  float r = length(gl_PointCoord * 2.0 - 1.0) * vPx * 0.5;
  float R = vPx * 0.5 - 0.5 * uDpr;
  float a = clamp(R - r + 0.5, 0.0, 1.0);
  if (a <= 0.0) discard;
  vec3 c = vC * (1.12 - 0.3 * r / max(R, 1.0));
  gl_FragColor = vec4(c, vA * a);
}`,hg=`
attribute vec3 p0; attribute vec3 color; attribute float size; attribute vec2 pxr; attribute float alpha; attribute float a0; attribute float pulse;
uniform float uMix, uScale, uDpr, uFogN, uFogF, uTime;
varying vec3 vC; varying float vA;
void main() {
  vec3 p = mix(p0, position, uMix);
  vec4 mv = modelViewMatrix * vec4(p, 1.0);
  float d = max(1.0, -mv.z);
  float px = clamp(size * uScale / d, pxr.x * uDpr, pxr.y * uDpr);
  float k = 1.0 + pulse * 0.22 * sin(uTime * 2.1 + p.x * 0.05);
  gl_PointSize = px * 3.6 * k;
  gl_Position = projectionMatrix * mv;
  vC = color;
  vA = alpha * mix(a0, 1.0, uMix) * (0.5 + 0.5 * (1.0 - smoothstep(uFogN, uFogF, d)));
}`,ug=`
varying vec3 vC; varying float vA;
void main() {
  float r = length(gl_PointCoord * 2.0 - 1.0);
  if (r > 1.0) discard;
  float a = pow(1.0 - r, 2.2) * 0.55;
  gl_FragColor = vec4(vC * a * vA, 1.0);
}`,dg=`
attribute vec3 p0; attribute vec3 color; attribute float size; attribute vec2 pxr; attribute float alpha; attribute vec2 ring;
uniform float uMix, uScale, uDpr;
varying vec3 vC; varying float vA; varying float vOut; varying float vRc; varying float vTh;
void main() {
  vec4 mv = modelViewMatrix * vec4(mix(p0, position, uMix), 1.0);
  float d = max(1.0, -mv.z);
  float px = clamp(size * uScale / d, pxr.x * uDpr, pxr.y * uDpr);
  vTh = ring.y * uDpr;
  vRc = px * 0.5 + ring.x * uDpr + vTh * 0.5;
  vOut = vRc + vTh * 0.5 + uDpr;
  gl_PointSize = vOut * 2.0;
  gl_Position = projectionMatrix * mv;
  vC = color; vA = alpha;
}`,fg=`
varying vec3 vC; varying float vA; varying float vOut; varying float vRc; varying float vTh;
void main() {
  float r = length(gl_PointCoord * 2.0 - 1.0) * vOut;
  float a = clamp(vTh * 0.5 - abs(r - vRc) + 0.5, 0.0, 1.0);
  if (a <= 0.0) discard;
  gl_FragColor = vec4(vC, vA * a);
}`,pg=`
attribute vec3 a0p; attribute vec3 c0p; attribute vec3 b0p; attribute vec3 a1p; attribute vec3 c1p; attribute vec3 b1p;
attribute vec3 ecol; attribute vec3 eprop; attribute float born;
uniform float uMix, uDpr, uFogN, uFogF; uniform vec2 uRes;
varying vec3 vC; varying float vA; varying float vE; varying float vHW;
vec3 bez(float t, vec3 a, vec3 c, vec3 b) { float u = 1.0 - t; return u * u * a + 2.0 * u * t * c + t * t * b; }
vec4 clip(vec3 p) { vec4 mv = modelViewMatrix * vec4(p, 1.0); mv.z = min(mv.z, -1.0); return projectionMatrix * mv; }
void main() {
  vec3 a = mix(a0p, a1p, uMix), c = mix(c0p, c1p, uMix), b = mix(b0p, b1p, uMix);
  float t = position.x;
  vec3 p = bez(t, a, c, b);
  vec4 mv = modelViewMatrix * vec4(p, 1.0);
  float behind = step(-1.0, mv.z);
  vec4 q = clip(p);
  vec4 q1 = clip(bez(clamp(t - 0.02, 0.0, 1.0), a, c, b)), q2 = clip(bez(clamp(t + 0.02, 0.0, 1.0), a, c, b));
  vec2 s1 = q1.xy / q1.w * uRes, s2 = q2.xy / q2.w * uRes;
  vec2 dir = s2 - s1; float l = length(dir); dir = l > 1e-5 ? dir / l : vec2(1.0, 0.0);
  vec2 nrm = vec2(-dir.y, dir.x);
  vHW = eprop.y * uDpr * 0.5;
  float hw = vHW + uDpr;
  q.xy += nrm * position.y * hw / uRes * q.w;
  gl_Position = q;
  vE = position.y * hw;
  vC = ecol;
  float d = -mv.z;
  vA = (1.0 - behind) * eprop.x * mix(born, 1.0, uMix) * mix(eprop.z, 1.0, t) * (0.35 + 0.65 * (1.0 - smoothstep(uFogN, uFogF, d)));
}`,mg=`
varying vec3 vC; varying float vA; varying float vE; varying float vHW;
void main() {
  float a = clamp(vHW - abs(vE) + 0.5, 0.0, 1.0);
  if (a <= 0.0) discard;
  gl_FragColor = vec4(vC, vA * a);
}`,gg=`
attribute vec3 p0; attribute vec3 color; attribute float alpha; attribute float a0;
uniform float uMix, uFogN, uFogF;
varying vec3 vC; varying float vA;
void main() {
  vec4 mv = modelViewMatrix * vec4(mix(p0, position, uMix), 1.0);
  gl_Position = projectionMatrix * mv;
  vC = color; vA = alpha * mix(a0, 1.0, uMix) * (0.4 + 0.6 * (1.0 - smoothstep(uFogN, uFogF, -mv.z)));
}`,_g=`
varying vec3 vC; varying float vA;
void main() { gl_FragColor = vec4(vC, vA); }`,xg=`
varying vec2 vU;
void main() { vU = position.xz; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }`,vg=`
uniform float uR; varying vec2 vU;
void main() {
  float r = length(vU) / uR;
  if (r > 1.0) discard;
  float a = 0.07 * (1.0 - smoothstep(0.55, 1.0, r)) + 0.025;
  gl_FragColor = vec4(0.47, 0.59, 0.77, a * (1.0 - smoothstep(0.96, 1.0, r)));
}`,Ir=[0.984,0.573,0.235],Pr=[0.176,0.831,0.749],yg=[0.38,0.8,0.78],Mh=[1,1,1],Mg=[0.66,0.75,0.88],Sg=[0.77,0.65,1];function Lr(t,e,n,i){let s=e*Math.min(n,1-n),r=(a)=>{let o=(a+t/30)%12;return n-s*Math.max(-1,Math.min(o-3,9-o,1))};return i[0]=r(0),i[1]=r(8),i[2]=r(4),i}var Sh=(t)=>Math.max(0,Math.min(1,t)),Ze=(t,e,n)=>t+(e-t)*n,bh=(t)=>t*t*(3-2*t),Nr=[0,0,0],qi={[pe.Child]:[2.4,16],[pe.Root]:[4.6,30],[pe.Lone]:[2.6,14],[pe.Work]:[6,26],[pe.Need]:[7,28]};function bg(t,e){gh();let n=e.instant,i=new zo({antialias:!0,alpha:!1,powerPreference:"high-performance",preserveDrawingBuffer:n});i.setClearColor(og,1);let s=i.domElement;s.tabIndex=0,s.setAttribute("aria-label","The map in 3D: places on the ground, conversations by recency, their spawned threads around them"),s.className="spacecanvas";let r=document.createElement("div");r.className="spacelabels",t.append(s,r);let a=new ur,o=new $e(wh,1,1,1e4),l={uMix:{value:1},uScale:{value:800},uDpr:{value:1},uFogN:{value:1000},uFogF:{value:4000},uTime:{value:0}},c={value:{x:1,y:1}},p=(A,C,F=!1,K={})=>new vn({uniforms:{...l,uRes:c,...K},vertexShader:A,fragmentShader:C,transparent:!0,depthWrite:!1,depthTest:!1,blending:F?ss:oi}),h=Ho(e.model(),null,Date.now()),u=new Float32Array(0),d=new Float32Array(0),v=new Float32Array(0),w=-1,f=0,m=-1,M=0,T=0,S=1,E=!1,P=!1,g=!1,y=!0,D=!0,q=0,N,X=null,rt=0,R=performance.now(),j=-1,tt=-1,G=!1,pt=null,J=!1,st=typeof matchMedia==="function"&&matchMedia("(prefers-reduced-motion: reduce)").matches,k={tx:0,ty:Qn*0.36,tz:0,dist:2000,theta:0.62,phi:0.98},It=0,nt=0,ct=null,yt={theta:0,phi:0},Gt=0,zt=0,Dt=p(lg,cg),ue=p(hg,ug,!0),ee=p(dg,fg),Jt=p(pg,mg);Jt.side=rn;let ve=p(gg,_g),ie={uR:{value:1}},Me=p(xg,vg,!1,ie);Me.side=rn;let ge=new Hi(new Ee,Dt),Re=new Hi(new Ee,ue),Le=new Hi(new Ee,ee),_e=new hn(new fs,Jt),be=new gr(new Ee,ve),Ue=new Ee,Fe=new hn(Ue,Me);Fe.renderOrder=0,be.renderOrder=1,_e.renderOrder=2,Re.renderOrder=3,ge.renderOrder=4,Le.renderOrder=5;for(let A of[Fe,be,_e,Re,ge,Le])A.frustumCulled=!1,a.add(A);let Je=new Float32Array((Ms+1)*2*3),U=[];for(let A=0;A<=Ms;A++)if(Je.set([A/Ms,-1,0,A/Ms,1,0],A*6),A<Ms){let C=A*2;U.push(C,C+1,C+3,C,C+3,C+2)}let Ye=new Int32Array(0),de=new Int32Array(0),xe=0,at=(A,C,F,K)=>A.setAttribute(C,new Ke(F,K)),re=(A,C,F,K)=>A.setAttribute(C,new pr(F,K)),kt=(A,C)=>{A.geometry.dispose(),A.geometry=C};function Xt(A,C,F,K,O,ot,xt,Tt,Ct){let Ut=K-A,lt=ot-F,$t=Math.hypot(Ut,lt)+0.001,Lt=$t+Math.abs(O-C)*0.3;Tt[Ct]=(A+K)/2-lt/$t*Lt*0.14*xt,Tt[Ct+1]=(C+O)/2+Lt*0.05,Tt[Ct+2]=(F+ot)/2+Ut/$t*Lt*0.14*xt}let Oe=(A)=>{let C=0;for(let F=0;F<A.length;F++)C=C*31+A.charCodeAt(F)|0;return C&1?1:-1};function b(A){y=!1;let C=Date.now(),F=h,K=u;h=Ho(e.model(),e.layout(),C);let O=h.items.length,ot=new Float32Array(O*3);d=new Float32Array(O*3);let xt=new Float32Array(O);for(let jt=0;jt<O;jt++){let he=h.items[jt];ot[jt*3]=he.x,ot[jt*3+1]=he.y,ot[jt*3+2]=he.z;let Ge=A?F.byId.get(he.t.id):void 0;if(Ge!==void 0&&Ge*3+2<K.length)d[jt*3]=K[Ge*3],d[jt*3+1]=K[Ge*3+1],d[jt*3+2]=K[Ge*3+2],xt[jt]=1;else d[jt*3]=he.x,d[jt*3+1]=he.y,d[jt*3+2]=he.z,xt[jt]=A?0:1}u=new Float32Array(d),v=new Float32Array(O*4),w=-1;let Tt=new Ee;at(Tt,"position",ot,3),at(Tt,"p0",d,3),at(Tt,"a0",xt,1),at(Tt,"color",new Float32Array(O*3),3),at(Tt,"alpha",new Float32Array(O),1);let Ct=new Float32Array(O),Ut=new Float32Array(O*2);h.items.forEach((jt,he)=>{Ct[he]=jt.size;let Ge=qi[jt.kind];Ut[he*2]=Ge[0],Ut[he*2+1]=Ge[1]}),at(Tt,"size",Ct,1),at(Tt,"pxr",Ut,2),Tt.setIndex(_()),kt(ge,Tt);let lt=[];h.items.forEach((jt,he)=>{if(jt.kind===pe.Work||jt.kind===pe.Need)lt.push(he)}),de=Int32Array.from(lt);let $t=new Ee,Lt=new Float32Array(lt.length*3),se=new Float32Array(lt.length*3),wt=new Float32Array(lt.length),gt=new Float32Array(lt.length*2),Ht=new Float32Array(lt.length*3),vt=new Float32Array(lt.length),Nt=new Float32Array(lt.length);lt.forEach((jt,he)=>{let Ge=h.items[jt];Lt.set(ot.subarray(jt*3,jt*3+3),he*3),se.set(d.subarray(jt*3,jt*3+3),he*3),wt[he]=Ge.size,gt[he*2]=qi[Ge.kind][0],gt[he*2+1]=qi[Ge.kind][1],Ht.set(Ge.kind===pe.Need?Ir:Pr,he*3),vt[he]=Ge.kind===pe.Work?1:0,Nt[he]=xt[jt]}),J=lt.some((jt)=>h.items[jt].kind===pe.Work),at($t,"position",Lt,3),at($t,"p0",se,3),at($t,"size",wt,1),at($t,"pxr",gt,2),at($t,"color",Ht,3),at($t,"pulse",vt,1),at($t,"a0",Nt,1),at($t,"alpha",new Float32Array(lt.length),1),kt(Re,$t);let Kt=[];h.items.forEach((jt,he)=>{if(jt.parent>=0)Kt.push(he)}),Ye=Int32Array.from(Kt);let qt=Kt.length,te=new Float32Array(qt*3),Qt=new Float32Array(qt*3),Ie=new Float32Array(qt*3),Te=new Float32Array(qt*3),Ji=new Float32Array(qt*3),Xo=new Float32Array(qt*3),qo=new Float32Array(qt);Kt.forEach((jt,he)=>{let Ge=h.items[jt],pn=Ge.parent,Yo=Oe(Ge.t.id);Te.set(ot.subarray(pn*3,pn*3+3),he*3),Xo.set(ot.subarray(jt*3,jt*3+3),he*3),Xt(ot[pn*3],ot[pn*3+1],ot[pn*3+2],ot[jt*3],ot[jt*3+1],ot[jt*3+2],Yo,Ji,he*3),te.set(d.subarray(pn*3,pn*3+3),he*3),Ie.set(d.subarray(jt*3,jt*3+3),he*3),Xt(d[pn*3],d[pn*3+1],d[pn*3+2],d[jt*3],d[jt*3+1],d[jt*3+2],Yo,Qt,he*3),qo[he]=Math.min(xt[pn],xt[jt])});let tn=new fs;tn.setAttribute("position",new Ke(Je,3)),tn.setIndex(U),re(tn,"a0p",te,3),re(tn,"c0p",Qt,3),re(tn,"b0p",Ie,3),re(tn,"a1p",Te,3),re(tn,"c1p",Ji,3),re(tn,"b1p",Xo,3),re(tn,"ecol",new Float32Array(qt*3),3),re(tn,"eprop",new Float32Array(qt*3),3),re(tn,"born",qo,1),tn.instanceCount=qt,kt(_e,tn),ht(F,A),xe=h.items.length*2+3;let Rn=new Ee;if(at(Rn,"position",new Float32Array(xe*3),3),at(Rn,"p0",new Float32Array(xe*3),3),at(Rn,"color",new Float32Array(xe*3),3),at(Rn,"size",new Float32Array(xe),1),at(Rn,"pxr",new Float32Array(xe*2),2),at(Rn,"alpha",new Float32Array(xe),1),at(Rn,"ring",new Float32Array(xe*2),2),Rn.setDrawRange(0,0),kt(Le,Rn),l.uMix.value=A&&!n?0:1,X=l.uMix.value<1?{t0:performance.now()}:null,!X)Bt(1);j=-1,D=!0,m=-1}function _(){let A=[];for(let C of[[pe.Child,pe.Lone,pe.Root],[pe.Work],[pe.Need]])h.items.forEach((F,K)=>{if(C.includes(F.kind))A.push(K)});return A}let B=new Map;function ht(A,C){let F=[],K=[],O=[],ot=[],xt=[],Tt=new Map,Ct=be.geometry.getAttribute("position"),Ut=be.geometry.getAttribute("p0"),lt=l.uMix.value,$t=(vt)=>Ct&&Ut?Ze(Ut.array[vt],Ct.array[vt],lt):0,Lt=(vt,Nt,Kt,qt)=>{let te=C?B.get(vt):void 0,Qt=F.length;for(let Te=0;Te<Nt.length;Te++)F.push(Nt[Te]),K.push(te&&te.length===Nt.length&&Ct?$t(te[Te]):Nt[Te]);let Ie=[];for(let Te=0;Te<Nt.length;Te++)Ie.push(Qt+Te);Tt.set(vt,Ie);for(let Te=0;Te<Nt.length/3;Te++)O.push(Kt[0],Kt[1],Kt[2]),ot.push(qt),xt.push(te||!C?1:0)},se=(vt,Nt,Kt,qt,te)=>{let Qt=[];for(let Ie=0;Ie<te;Ie++){let Te=Ie/te*Math.PI*2,Ji=(Ie+1)/te*Math.PI*2;Qt.push(vt+Math.cos(Te)*qt,Nt,Kt+Math.sin(Te)*qt,vt+Math.cos(Ji)*qt,Nt,Kt+Math.sin(Ji)*qt)}return Qt},wt=h.radius*1.03;for(let vt of h.scale.levels){let Nt=vt.days===0;Lt("lv:"+vt.days,se(0,vt.y,0,wt,128),Nt?Pr:[0.36,0.45,0.58],Nt?0.26:0.13)}for(let vt of h.places){let Nt=vt.p.need>0;if(Lt("pl:"+vt.p.key,se(vt.cx,0.5,vt.cz,vt.r,Math.max(24,Math.min(96,Math.round(vt.r/3)))),Nt?Ir:[0.5,0.6,0.75],Nt?0.42:0.26),vt.r>14&&vt.top>0)Lt("col:"+vt.p.key,[vt.cx,0,vt.cz,vt.cx,vt.top,vt.cz],[0.5,0.6,0.75],0.07)}B=Tt;let gt=new Ee;at(gt,"position",Float32Array.from(F),3),at(gt,"p0",Float32Array.from(K),3),at(gt,"color",Float32Array.from(O),3),at(gt,"alpha",Float32Array.from(ot),1),at(gt,"a0",Float32Array.from(xt),1),kt(be,gt);let Ht=new Float32Array([-wt,0,-wt,wt,0,-wt,wt,0,wt,-wt,0,wt]);Ue.setAttribute("position",new Ke(Ht,3)),Ue.setIndex([0,2,1,0,3,2]),ie.uR.value=wt}function it(A){if(!A||A.kind==="place")return-1;let C=A.kind==="effort"?A.id:e.model().byId.get(A.id)?.rootId;return(C?h.eIdx.get(C):void 0)??-1}function mt(){D=!1;let A=Date.now(),C=e.sel(),F=it(C),K=it(e.hovered());tt=F>=0?F:K,G=F>=0;let O=tt<0?1:G?0.22:0.5,ot=ge.geometry,xt=ot.getAttribute("color").array,Tt=ot.getAttribute("alpha").array;h.items.forEach((Lt,se)=>{let wt=Lt.t,gt=Sh((A-wt.e)/_i/14),Ht,vt;if(Lt.kind===pe.Need)Ht=Ir,vt=1;else if(Lt.kind===pe.Work)Ht=Pr,vt=1;else if(Lt.kind===pe.Root)Ht=Lr(214,0.42,Ze(0.8,0.5,gt),Nr),vt=Ze(1,0.75,gt);else if(Lt.kind===pe.Lone)Ht=Lr(216,0.22,Ze(0.58,0.36,gt),Nr),vt=Ze(0.62,0.38,gt);else Ht=Lr(214,0.4,Ze(0.7,0.45,gt),Nr),vt=Ze(0.95,0.6,gt);if(wt.synthetic)vt*=0.45;if(vt*=Math.max(0.1,wt.fade),tt>=0&&Lt.eff!==tt)vt*=Lt.kind>=pe.Work?Math.max(O,0.55):O;xt[se*3]=Ht[0],xt[se*3+1]=Ht[1],xt[se*3+2]=Ht[2],Tt[se]=vt}),ot.getAttribute("color").needsUpdate=!0,ot.getAttribute("alpha").needsUpdate=!0;let Ct=Re.geometry.getAttribute("alpha");if(Ct)de.forEach((Lt,se)=>{let wt=h.items[Lt];Ct.array[se]=Math.max(0.15,wt.t.fade)*(tt>=0&&wt.eff!==tt?0.55:1)}),Ct.needsUpdate=!0;let Ut=_e.geometry,lt=Ut.getAttribute("ecol"),$t=Ut.getAttribute("eprop");if(lt&&$t){let Lt=lt.array,se=$t.array;Ye.forEach((wt,gt)=>{let Ht=h.items[wt],vt=Ht.t,Nt=Sh(1-(A-vt.e)/(_i*10)),Kt=vt.need||vt.working,qt=Kt?0.95:0.55+0.35*Nt,te=Kt?2.4:1.6+0.7*Nt;if(tt>=0)if(Ht.eff===tt)qt=Math.min(1,qt*1.35+0.25),te*=1.35;else qt*=G?Kt?0.4:0.14:0.4;qt*=Math.max(0.1,vt.fade);let Qt=vt.need?Ir:vt.working?Pr:Lr(213,0.7,Ze(0.66,0.8,Nt),Nr);Lt[gt*3]=Qt[0],Lt[gt*3+1]=Qt[1],Lt[gt*3+2]=Qt[2],se[gt*3]=qt,se[gt*3+1]=te,se[gt*3+2]=Kt?0.8:0.65}),lt.needsUpdate=!0,$t.needsUpdate=!0}Mt(),m=-1}function Mt(){let A=Le.geometry,C=A.getAttribute("position");if(!C)return;let F=C.array,K=A.getAttribute("p0").array,O=A.getAttribute("color").array,ot=A.getAttribute("size").array,xt=A.getAttribute("pxr").array,Tt=A.getAttribute("alpha").array,Ct=A.getAttribute("ring").array,Ut=0,lt=(gt,Ht,vt,Nt,Kt)=>{if(gt<0||Ut>=xe)return;let qt=h.items[gt];F[Ut*3]=qt.x,F[Ut*3+1]=qt.y,F[Ut*3+2]=qt.z,K[Ut*3]=d[gt*3],K[Ut*3+1]=d[gt*3+1],K[Ut*3+2]=d[gt*3+2],O[Ut*3]=Ht[0],O[Ut*3+1]=Ht[1],O[Ut*3+2]=Ht[2],ot[Ut]=qt.size,xt[Ut*2]=qi[qt.kind][0],xt[Ut*2+1]=qi[qt.kind][1],Tt[Ut]=vt,Ct[Ut*2]=Nt,Ct[Ut*2+1]=Kt,Ut++};if(e.stats().unreadOn)h.items.forEach((gt,Ht)=>{if(gt.t.unread&&gt.t.fade>0.5&&(tt<0||gt.eff===tt))lt(Ht,Mg,0.22,1.5,1)});let $t=e.picked?.();if($t?.size)for(let gt of $t)lt(h.byId.get(gt)??-1,Sg,0.9,9,1.5);let Lt=e.opened();lt(Lt?h.byId.get(Lt)??-1:-1,yg,0.9,6,1.5),lt(j,Mh,0.55,3,1);let se=e.sel(),wt=se&&se.kind!=="place"?se.kind==="effort"?h.efforts[h.eIdx.get(se.id)??-1]?.rootIdx??-1:h.byId.get(se.id)??-1:-1;lt(wt,Mh,0.95,3,1.5);for(let gt of["position","p0","color","size","pxr","alpha","ring"])A.getAttribute(gt).needsUpdate=!0;A.setDrawRange(0,Ut)}function Y(A){let C=M&&T?M/T:1.6;return A/(xi*Math.min(1,C))*1.08}function et(){return{tx:0,ty:Qn*0.3,tz:0,dist:Y(h.radius)*0.92,theta:0.62,phi:0.98}}function dt(){let A=Math.sin(k.phi),C=Math.cos(k.phi),F=Math.sin(k.theta),K=Math.cos(k.theta);if(o.position.set(k.tx+k.dist*A*F,k.ty+k.dist*C,k.tz+k.dist*A*K),o.up.set(-C*F,A,-C*K),o.lookAt(k.tx,k.ty,k.tz),o.near=Math.max(1,k.dist*0.02),o.far=k.dist*4+4000,Math.abs(It)>0.5&&M&&T)o.setViewOffset(M,T,It,0,M,T);else o.clearViewOffset();o.updateProjectionMatrix(),o.updateMatrixWorld(),l.uFogN.value=k.dist*0.6,l.uFogF.value=k.dist*2.6,f++}let I=(A)=>Math.atan2(Math.sin(A),Math.cos(A));function Q(A,C=900){if(n||C<=0){ct=null,Object.assign(k,A),dt(),we();return}let F={...k};A={...A,theta:F.theta+I(A.theta-F.theta)},ct={t0:performance.now(),ms:C,from:F,to:A},yt.theta=yt.phi=0,we()}function Z(A){if(!ct)return!1;let C=Math.min(1,(A-ct.t0)/ct.ms),F=bh(C),K=ct.from,O=ct.to;if(k.tx=Ze(K.tx,O.tx,F),k.ty=Ze(K.ty,O.ty,F),k.tz=Ze(K.tz,O.tz,F),k.dist=Math.exp(Ze(Math.log(K.dist),Math.log(O.dist),F)),k.theta=Ze(K.theta,O.theta,F),k.phi=Ze(K.phi,O.phi,F),C>=1)ct=null;return!0}let ut=(A)=>({...A,theta:k.theta,phi:Math.min(Math.max(k.phi,0.35),1.15)});function ft(A,C){let F=h.efforts[A];if(!F)return;let K=F.items.length<=1,O=h.items[F.rootIdx];if(K)Q(ut({tx:O.x,ty:O.y,tz:O.z,dist:190}),C);else Q(ut({tx:F.cx,ty:F.cy,tz:F.cz,dist:Math.max(150,Y(F.ext+12)*1.15)}),C)}function St(A,C){if(A.kind==="place"){let F=h.places.find((K)=>K.p.key===A.id);if(F)Q(ut({tx:F.cx,ty:F.top*0.45,tz:F.cz,dist:Math.max(260,Y(F.r)*1.25)}),C);return}ft(it(A),C)}function Bt(A){let C=ge.geometry.getAttribute("position")?.array;if(!C||C.length!==u.length)return;for(let F=0;F<u.length;F++)u[F]=Ze(d[F],C[F],A)}function Ft(){if(w===f)return;w=f;let A=o.matrixWorldInverse.elements,C=o.projectionMatrix.elements,F=h.items.length,K=T/(2*xi);for(let O=0;O<F;O++){let ot=u[O*3],xt=u[O*3+1],Tt=u[O*3+2],Ct=A[0]*ot+A[4]*xt+A[8]*Tt+A[12],Ut=A[1]*ot+A[5]*xt+A[9]*Tt+A[13],lt=A[2]*ot+A[6]*xt+A[10]*Tt+A[14],$t=C[0]*Ct+C[4]*Ut+C[8]*lt+C[12],Lt=C[1]*Ct+C[5]*Ut+C[9]*lt+C[13],se=C[3]*Ct+C[7]*Ut+C[11]*lt+C[15],wt=-lt,gt=O*4;if(wt<1||se<=0){v[gt]=-1e5,v[gt+1]=-1e5,v[gt+2]=-1,v[gt+3]=0;continue}v[gt]=($t/se*0.5+0.5)*M,v[gt+1]=(-Lt/se*0.5+0.5)*T,v[gt+2]=wt;let Ht=h.items[O],vt=qi[Ht.kind];v[gt+3]=Math.min(vt[1],Math.max(vt[0],Ht.size*K/wt))/2}}function Et(A,C){Ft();let F=-1,K=1e9,O=ge.geometry.getAttribute("alpha")?.array;for(let ot=0;ot<h.items.length;ot++){let xt=ot*4;if(v[xt+2]<0)continue;if(O&&O[ot]<0.08)continue;let Tt=v[xt]-A,Ct=v[xt+1]-C,Ut=Math.hypot(Tt,Ct),lt=Math.max(7,v[xt+3]+4);if(Ut>lt)continue;let $t=h.items[ot].kind>=pe.Work?0.85:1,Lt=Ut/lt*$t;if(Lt<K)K=Lt,F=ot}return F}let Ot=(A)=>({kind:A.t.rootId===A.t.id?"effort":"thread",id:A.t.id}),Vt=new Map,At=[];function ce(A,C,F,K,O=!1){let ot=Vt.get(A);if(!ot){let xt=document.createElement("div");xt.className="slab "+F,xt.textContent=C,xt.style.opacity="0",r.appendChild(xt);let Tt=Xi(C)*K+8;ot={el:xt,o:0,target:0,text:C,cls:F,sx:0,sy:0,level:O,cand:{key:A,x:0,y:0,w:Tt,h:K*1.3,prio:0,shown:!1,force:!1,on:!1}},Vt.set(A,ot)}else if(ot.text!==C)ot.text=C,ot.el.textContent=C,ot.cand.w=Xi(C)*K+8;return ot}let Ce=(A,C,F)=>_h(A.title||A.id,C,F);function dn(){m=f,Ft(),At.length=0;let A={x:4,y:4,w:M-8-e.cardWidth()*(e.sel()?1:0),h:T-30};for(let wt of Vt.values())wt.target=0,wt.cand.shown=wt.o>0.5,wt.cand.force=!1;let C=e.sel(),F=C&&C.kind!=="place"?C.kind==="effort"?h.efforts[h.eIdx.get(C.id)??-1]?.rootIdx??-1:h.byId.get(C.id)??-1:-1,K=(wt,gt,Ht)=>{let vt=h.items[wt],Nt=wt*4;if(v[Nt+2]<0)return;let Kt=vt.root,qt=Kt?12.5:11.5,te=vt.kind===pe.Need?" need":vt.kind===pe.Work?" work":"",Qt=ce("t:"+vt.t.id,Ce(vt.t,qt,Kt?280:220),(Kt?"root":"kid")+te,qt);Qt.sx=v[Nt],Qt.sy=v[Nt+1]-v[Nt+3]-3,Qt.cand.x=Qt.sx,Qt.cand.y=Qt.sy,Qt.cand.prio=gt,Qt.cand.force=Ht,At.push(Qt.cand)};if(tt>=0){let wt=h.efforts[tt];for(let gt of wt.items){let Ht=h.items[gt];K(gt,(Ht.root?10:6)+(Ht.kind>=pe.Work?2:0)+Math.min(1,Ht.t.w),gt===F||gt===j)}}else if(j>=0)K(j,20,!0);for(let wt=0;wt<h.efforts.length;wt++){let gt=h.efforts[wt];if(wt===tt||gt.rootIdx<0||gt.e.fade<0.3)continue;if(gt.score<0.4&&!gt.e.need)continue;K(gt.rootIdx,gt.score,!1)}let O=Math.sin(k.theta),ot=Math.cos(k.theta),xt=o.matrixWorldInverse.elements,Tt=o.projectionMatrix.elements,Ct=(wt,gt,Ht)=>{let vt=xt[0]*wt+xt[4]*gt+xt[8]*Ht+xt[12],Nt=xt[1]*wt+xt[5]*gt+xt[9]*Ht+xt[13],Kt=xt[2]*wt+xt[6]*gt+xt[10]*Ht+xt[14],qt=Tt[0]*vt+Tt[4]*Nt+Tt[8]*Kt+Tt[12],te=Tt[1]*vt+Tt[5]*Nt+Tt[9]*Kt+Tt[13],Qt=Tt[3]*vt+Tt[7]*Nt+Tt[11]*Kt+Tt[15];if(Qt<=1)return null;return[(qt/Qt*0.5+0.5)*M,(-te/Qt*0.5+0.5)*T]},Ut=k.phi>0.25?1:k.phi/0.25;for(let wt of h.places){let gt=Ct(wt.cx+O*wt.r*Ut,0,wt.cz+ot*wt.r*Ut);if(!gt)continue;let Ht=13,vt=ce("p:"+wt.p.key,wt.p.name,"place"+(wt.p.need?" need":""),Ht);vt.sx=gt[0],vt.sy=gt[1]+Ht*1.3+3,vt.cand.x=vt.sx,vt.cand.y=vt.sy,vt.cand.prio=3+Math.min(2,Math.log10(1+wt.p.count))+(C?.kind==="place"&&C.id===wt.p.key?20:0),vt.cand.force=!1,At.push(vt.cand)}let lt=Math.cos(k.theta),$t=Math.sin(k.theta),Lt=Math.max(40,(M-(e.sel()?e.cardWidth():0))/2-110)*(2*k.dist*xi/T),se=-1e9;for(let wt of h.scale.levels){let gt=ce("lv:"+wt.days,wt.label,"level"+(wt.days===0?" today":""),11,!0);if(k.phi<0.2)continue;let Ht=Ct(k.tx+lt*Lt,wt.y,k.tz-$t*Lt);if(!Ht)continue;let vt=Math.min(Ht[0],M-75-(e.sel()?e.cardWidth():0));if(vt>0&&Ht[1]>10&&Ht[1]<T-30&&Ht[1]-se>15)gt.sx=vt,gt.sy=Ht[1]+7,se=Ht[1],gt.cand.w=Xi(wt.label)*11+24,gt.cand.x=vt+gt.cand.w/2,gt.cand.y=gt.sy-14+gt.cand.h,gt.cand.force=!0,gt.cand.prio=100,At.push(gt.cand)}yh(At,ig+h.places.length,A,6);for(let wt of At)if(wt.on)Vt.get(wt.key).target=1}function Yi(A){let C=!1,F=n?1:1-Math.exp(-A/110);for(let[K,O]of Vt){let ot=O.o+(O.target-O.o)*F;if(O.o=Math.abs(ot-O.target)<0.02?O.target:ot,O.o!==O.target)C=!0;if(O.o<=0){if(O.el.style.display!=="none")O.el.style.display="none";if(O.target===0&&!h.byId.has(K.slice(2))&&K.startsWith("t:"))O.el.remove(),Vt.delete(K);continue}if(O.el.style.display==="none")O.el.style.display="";O.el.style.opacity=O.o.toFixed(3),O.el.style.transform=O.level?`translate(${O.sx.toFixed(1)}px,${(O.sy-14).toFixed(1)}px)`:`translate(${(O.sx-O.cand.w/2+4).toFixed(1)}px,${(O.sy-O.cand.h).toFixed(1)}px)`}return C}function we(){if(!E||P||q||typeof document<"u"&&document.hidden)return;if(N)clearTimeout(N),N=void 0;q=requestAnimationFrame(Tn)}let jn=()=>performance.now()-R,Ss=()=>!n&&!st&&!e.sel()&&!ct&&jn()>Wo&&jn()<ag&&ze.size===0;function Tn(A){if(q=0,!E||P||!M)return;let C=performance.now(),F=rt?Math.min(100,A-rt):16;if(rt=A,y)b(g&&h.items.length>0);if(D)mt();let K=!1;if(Z(C))K=!0;if(Math.abs(yt.theta)+Math.abs(yt.phi)>0.00001&&ze.size===0){k.theta+=yt.theta*F,k.phi=Math.min(Vo,Math.max(0.0005,k.phi+yt.phi*F));let O=Math.exp(-F/260);if(yt.theta*=O,yt.phi*=O,Math.abs(yt.theta)+Math.abs(yt.phi)<0.00001)yt.theta=yt.phi=0;K=!0}if(Ss())k.theta-=rg*F/1000,K=!0;if(nt=e.sel()?e.cardWidth()/2:0,Math.abs(It-nt)>0.5)It=n?nt:It+(nt-It)*(1-Math.exp(-F/140)),K=!0;else It=nt;if(X){let O=Math.min(1,(C-X.t0)/sg);if(l.uMix.value=bh(O),Bt(l.uMix.value),w=-1,m=-1,O>=1)X=null;K=!0}if(dt(),m!==f)dn();if(Yi(F))K=!0;if(l.uTime.value=A/1000,i.render(a,o),Gt++,zt=zt?zt*0.9+(performance.now()-C)*0.1:performance.now()-C,K)we();else if(J&&!st&&!n&&jn()<30000)N=setTimeout(()=>{N=void 0,we()},70);else rt=0;if(!K&&!e.sel()&&!n&&!st&&jn()<Wo){let O=Wo-(performance.now()-R);if(O>0&&!N)N=setTimeout(()=>{N=void 0,we()},O+20)}}function vi(){let A=t.getBoundingClientRect();if(!A.width||!A.height)return;let C=Math.round(A.width),F=Math.round(A.height),K=Math.min(window.devicePixelRatio||1,2);if(C===M&&F===T&&K===S)return;M=C,T=F,S=K,i.setPixelRatio(S),i.setSize(M,T,!1),s.style.width=M+"px",s.style.height=T+"px",o.aspect=M/T,l.uScale.value=T*S/(2*xi),l.uDpr.value=S,c.value.x=M*S/2,c.value.y=T*S/2,dt(),we()}let ze=new Map,ke=null,je=null,ti={t:0,x:0,y:0},En=()=>{R=performance.now(),ct=null},fn=(A)=>{let C=s.getBoundingClientRect();return{x:A.clientX-C.left,y:A.clientY-C.top}};function Zi(A,C){let F=2*k.dist*xi/T,K=Math.sin(k.theta),O=Math.cos(k.theta);k.tx+=-A*F*O-C*F*K,k.tz+=A*F*K-C*F*O}function ei(A,C,F){let K=Math.min(7000,Math.max(12,k.dist*A));A=K/k.dist;let O=k.dist*xi,ot=O*(M/T),xt=(C+It)/M*2-1,Tt=1-F/T*2,Ct=o.matrixWorld.elements,Ut=1-A;k.tx+=(Ct[0]*xt*ot+Ct[4]*Tt*O)*Ut,k.ty+=(Ct[1]*xt*ot+Ct[5]*Tt*O)*Ut,k.tz+=(Ct[2]*xt*ot+Ct[6]*Tt*O)*Ut,k.ty=Math.max(-50,Math.min(Qn*1.5,k.ty)),k.dist=K}function bs(A){if(s.setPointerCapture?.(A.pointerId),ze.set(A.pointerId,fn(A)),En(),yt.theta=yt.phi=0,ze.size===1){let C=fn(A);ke={mode:A.button===2||A.button===1||A.shiftKey||A.ctrlKey||A.metaKey?"pan":"orbit",x:C.x,y:C.y,moved:!1,t:performance.now(),button:A.button}}else if(ze.size===2){let[C,F]=[...ze.values()];if(je={d:Math.hypot(C.x-F.x,C.y-F.y),mx:(C.x+F.x)/2,my:(C.y+F.y)/2},ke)ke.moved=!0}if(j>=0&&A.pointerType==="touch")x(-1)}function Dr(A){let C=fn(A);if(!ze.has(A.pointerId)){if(A.pointerType!=="touch"){if(R=performance.now(),x(Et(C.x,C.y)),!q&&!N)we()}return}if(ze.set(A.pointerId,C),En(),ze.size>=2&&je){let[O,ot]=[...ze.values()],xt=Math.hypot(O.x-ot.x,O.y-ot.y),Tt=(O.x+ot.x)/2,Ct=(O.y+ot.y)/2;if(xt>0&&je.d>0)ei(je.d/xt,Tt,Ct);Zi(Tt-je.mx,Ct-je.my),je={d:xt,mx:Tt,my:Ct},we();return}if(!ke)return;let F=C.x-ke.x,K=C.y-ke.y;if(!ke.moved&&Math.hypot(F,K)<5)return;if(!ke.moved)ke.moved=!0,x(-1);if(ke.x=C.x,ke.y=C.y,ke.mode==="pan")Zi(F,K);else{let O=2.6/T;k.theta-=F*O,k.phi=Math.min(Vo,Math.max(0.0005,k.phi-K*O)),yt.theta=-F*O/16,yt.phi=-K*O/16}we()}function ws(A){let C=fn(A);if(ze.delete(A.pointerId),ze.size<2)je=null;let F=ke;if(ze.size===0)ke=null;if(!F||F.moved||performance.now()-F.t>600||F.button!==0&&A.pointerType==="mouse"){if(F&&F.mode==="pan")yt.theta=yt.phi=0;we();return}if(yt.theta=yt.phi=0,A.shiftKey&&e.togglePick){let Ct=Et(C.x,C.y);if(Ct>=0&&!h.items[Ct].t.synthetic)e.togglePick(Ot(h.items[Ct])),D=!0;we();return}let K=performance.now(),O=K-ti.t<380&&Math.hypot(C.x-ti.x,C.y-ti.y)<24;if(ti=O?{t:0,x:0,y:0}:{t:K,x:C.x,y:C.y},s.focus({preventScroll:!0}),O){let Ct=e.sel();if(Ct&&Ct.kind!=="place")e.open(Ct);return}let ot=Et(C.x,C.y);if(ot<0){e.select(null);return}let xt=h.items[ot],Tt=it(e.sel());if(e.select(Ot(xt)),xt.root||Tt!==xt.eff)ft(xt.eff)}function Ur(A){A.preventDefault(),En();let C=A.deltaMode===1?33:A.deltaMode===2?T:1,F=Math.exp(A.deltaY*C*(A.ctrlKey?0.01:0.0016)),K=fn(A);ei(F,K.x,K.y),we()}function As(A){if(A.key==="Enter"){let K=e.sel();if(K&&K.kind!=="place")e.open(K);A.preventDefault();return}let F=A.key;if(!["ArrowLeft","ArrowRight","ArrowUp","ArrowDown","+","=","-"].includes(F))return;if(A.preventDefault(),En(),F==="+"||F==="=")ei(0.85,M/2-It,T/2);else if(F==="-")ei(1.1764705882352942,M/2-It,T/2);else if(A.shiftKey)Zi(F==="ArrowLeft"?40:F==="ArrowRight"?-40:0,F==="ArrowUp"?40:F==="ArrowDown"?-40:0);else if(F==="ArrowLeft")k.theta+=0.08;else if(F==="ArrowRight")k.theta-=0.08;else k.phi=Math.min(Vo,Math.max(0.0005,k.phi+(F==="ArrowUp"?-0.08:0.08)));we()}function x(A){if(A===j)return;j=A,s.style.cursor=A>=0?"pointer":"grab",e.hover(A>=0?Ot(h.items[A]):null),D=!0,we()}let L=()=>{if(!ze.size)x(-1)},V=(A)=>A.preventDefault(),z=()=>{if(!document.hidden)we()},H=(A)=>{A.preventDefault()},bt=()=>{y=!0,we()};s.addEventListener("pointerdown",bs),s.addEventListener("pointermove",Dr),s.addEventListener("pointerup",ws),s.addEventListener("pointercancel",ws),s.addEventListener("pointerleave",L),s.addEventListener("wheel",Ur,{passive:!1}),s.addEventListener("keydown",As),s.addEventListener("contextmenu",V),s.addEventListener("webglcontextlost",H),s.addEventListener("webglcontextrestored",bt),document.addEventListener("visibilitychange",z);let Pt=new ResizeObserver(vi);Pt.observe(t),s.style.cursor="grab";let _t={update(A){if(A==="data")y=!0;D=!0,we()},selected(A){D=!0;let C=e.sel();if(C&&A!=="space")if(E&&!y)St(C);else pt=C;we()},redraw(){D=!0,we()},show(A){if(E=!0,vi(),y)b(!1);let C=!g;if(g=!0,A!==void 0){let F=e.layout(),K=!A||!F||A.w>=F.size*0.85,O=K?et():(()=>{let xt=Cr(F,A.x,A.y),Tt=A.w*zn/2;return{tx:xt.x,ty:Qn*0.3,tz:xt.z,dist:Math.max(120,Tt/(xi*(M/T))*1.05),theta:0,phi:0.9}})(),ot={...O,theta:K?0:O.theta,phi:0.0005,ty:Qn*0.3,dist:K?Y(h.radius)*1.05:O.dist/1.05};if(n)Object.assign(k,O),dt();else Object.assign(k,ot),dt(),Q(O,1100)}else if(C)Object.assign(k,et()),dt();if(pt){let F=pt,K=e.sel();if(pt=null,A===void 0&&K&&K.kind===F.kind&&K.id===F.id)St(F)}R=performance.now(),D=!0,m=-1,we()},hide(){if(E=!1,q)cancelAnimationFrame(q);if(q=0,N)clearTimeout(N);N=void 0,rt=0,x(-1)},destroy(){_t.hide(),P=!0,Pt.disconnect(),document.removeEventListener("visibilitychange",z);for(let A of[ge,Re,Le,_e,be,Fe])A.geometry.dispose(),A.material.dispose();i.dispose(),s.remove(),r.remove()},topDown(){En(),Q({tx:0,ty:Qn*0.3,tz:0,dist:Y(h.radius)*1.06,theta:0,phi:0.0005},1000)},overview(){En(),Q(et(),1000)},target(){let A=e.layout();return vh(A??{size:1000},k.tx,k.tz)},debug:{stats(){let A=i.info.render,C=0;for(let F of Vt.values())if(F.o>0.5)C++;return{drawCalls:A.calls,triangles:A.triangles,points:A.points,nodes:h.items.length,edges:Ye.length,edgeInstances:_e.geometry.instanceCount??0,labels:C,frameMs:zt,frames:Gt}},camera:()=>({target:[k.tx,k.ty,k.tz],dist:k.dist,theta:k.theta,phi:k.phi}),pick:(A,C)=>{let F=Et(A,C);return F>=0?Ot(h.items[F]):null},probe:()=>{let A=_e.geometry,C=(F)=>Array.from(A.getAttribute(F)?.array??[]).slice(0,6);return{ecol:C("ecol"),eprop:C("eprop"),a1p:C("a1p"),b1p:C("b1p"),c1p:C("c1p"),born:C("born"),uRes:c.value,mix:l.uMix.value}},labels:()=>[...Vt.entries()].filter(([,A])=>A.o>0.5).map(([A,C])=>A+"="+C.text)}};return _t}export{bg as createSpace};
