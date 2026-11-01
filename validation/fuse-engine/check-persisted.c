/* Independent query check of actual SST filter bytes using reference XXH3.
 * cc -O2 -I/path/to/xxHash-v0.8.3 check-persisted.c -o /tmp/check-persisted
 * /tmp/check-persisted old.sst new.sst ...; diagnostic, not an engine dependency.
 */
#define XXH_INLINE_ALL
#include "xxhash.h"
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static uint64_t le(const unsigned char *p, unsigned n) {
    uint64_t v=0; for(unsigned i=0;i<n;i++)v|=(uint64_t)p[i]<<(8*i); return v;
}
static uint64_t mix(uint64_t x) {
    x=(x^(x>>30))*UINT64_C(0xbf58476d1ce4e5b9);
    x=(x^(x>>27))*UINT64_C(0x94d049bb133111eb);return x^(x>>31);
}
static int contains(const unsigned char *b, int version, const void *key, size_t n) {
    if(version<5) {
        uint64_t h=UINT64_C(0xcbf29ce484222325);
        for(size_t i=0;i<n;i++)h=(h^((const unsigned char*)key)[i])*UINT64_C(0x100000001b3);
        uint64_t step=((h<<21)|(h>>43))|1, bits=le(b+1,8);
        for(unsigned i=0;i<b[0];i++) {uint64_t p=(h+i*step)%bits;if(!(b[9+p/8]&(1<<(p%8))))return 0;}
        return 1;
    }
    if(b[9]!=2) {fprintf(stderr,"expected actual fuse graph\n");exit(2);}
    uint64_t h=mix(XXH3_64bits(key,n)+le(b+24,8));
    uint64_t segment=le(b+32,4), count_length=le(b+36,4)*segment;
    uint64_t a=(uint64_t)(((__uint128_t)h*count_length)>>64), mask=segment-1;
    const unsigned char *fp=b+48;
    return (unsigned char)(fp[a]^fp[(a+segment)^(h&mask)]^fp[(a+2*segment)^((h>>16)&mask)]^fp[(a+3*segment)^((h>>32)&mask)])==(unsigned char)h;
}
int main(int argc,char **argv) {
    for(int arg=1;arg<argc;arg++) {
        FILE *f=fopen(argv[arg],"rb");if(!f)return 2;
        unsigned char footer[44];if(fseek(f,-44,SEEK_END)||fread(footer,1,44,f)!=44)return 2;
        uint64_t off=le(footer+20,8),len=le(footer+28,8);int version=(int)le(footer+16,4);
        unsigned char *b=malloc(len);if(!b||fseek(f,(long)off,SEEK_SET)||fread(b,1,len,f)!=len)return 2;fclose(f);
        uint64_t n=strstr(argv[arg],"-1000000/")?1000000:10000;
        uint64_t uniform=0,prefix=0,fn=0; char key[80];
        for(uint64_t i=0;i<n;i++) {
            int size=snprintf(key,sizeof key,"agent/%04"PRIu64"/memory/%020"PRIu64,i%64,UINT64_MAX-i);
            fn+=!contains(b,version,key,(size_t)size);
        }
        for(uint64_t i=0;i<1000000;i++) {
            uint64_t h=mix(i+20261016); unsigned char binary[38]={0};
            for(unsigned j=1;j<38;j++){h=mix(h);binary[j]=(unsigned char)h;}
            uniform+=contains(b,version,binary,sizeof binary);
            uint64_t id=mix(i+20261016)|UINT64_C(0x8000000000000000);
            int size=snprintf(key,sizeof key,"agent/%04"PRIu64"/memory/%020"PRIu64,id%64,UINT64_MAX-id);
            prefix+=contains(b,version,key,(size_t)size);
        }
        printf("{\"file\":\"%s\",\"version\":%d,\"members\":%"PRIu64",\"false_negatives\":%"PRIu64",\"misses_per_distribution\":1000000,\"uniform_positives\":%"PRIu64",\"prefix_positives\":%"PRIu64"}\n",argv[arg],version,n,fn,uniform,prefix);
        free(b);if(fn)return 1;
    }
}
