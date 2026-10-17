"""Restore sampled IPs when perf's DWARF unwinder emits an empty call chain."""
import json


def restore_ips(stacks, leaves, output, metadata):
    blocks=[b.splitlines() for b in stacks.read_text().split('\n\n') if b.strip()]
    ips=[line for line in leaves.read_text().splitlines() if line.strip()]
    assert len(blocks)==len(ips), (len(blocks),len(ips))
    restored=0
    periods=0
    event='cpu_core/cycles/u:'
    with output.open('w') as out:
        for block,ip in zip(blocks,ips):
            header,leaf=ip.split(event,1)
            original=block[0].split(event,1)[0]
            assert header.split()==original.split(), (header,original)
            periods+=int(header.split()[-1])
            if len(block)==1:
                assert leaf.strip()
                block.extend(['\t'+leaf.strip(), '\t0 unwound_callers_unavailable ([unknown])'])
                restored+=1
            out.write('\n'.join(block)+'\n\n')
    metadata.write_text(json.dumps(dict(samples=len(blocks),empty_chains_restored=restored,period_sum=periods),indent=2)+'\n')
    return periods
