#!/usr/bin/env python3
"""Debug packet wire parity, including invalid subscriptions, unknown sample kinds and long arrays."""
import json
import os
try:
    from . import lab, test_rust_parity as base, oracle_harness as harness
except ImportError:
    import lab
    import test_rust_parity as base
    import oracle_harness as harness


def main():
    build=lab.LOCAL/'debug-packet-codec-parity'; build.mkdir(exist_ok=True)
    (build/'report.json').unlink(missing_ok=True)
    lock,jars,libraries=base.verify_artifacts()
    cp=os.pathsep.join(map(str,[jars['server'],*libraries])); classes=build/'classes';classes.mkdir(exist_ok=True)
    source=lab.ROOT/'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaDebugPacketOracle.java'
    sources=[source,lab.ROOT/'crates/sunrise-protocol/src/packets/play.rs',lab.ROOT/'crates/sunrise-protocol/examples/debug_packet_probe.rs']
    tested=base.source_hashes(sources)
    run=lambda cmd,name:harness.run_logged(cmd,build,name)
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-cp',cp,'-d',classes,source],'compile-java')
    run(['java','-cp',str(classes)+os.pathsep+cp,'dev.sunriselab.parity.VanillaDebugPacketOracle',build/'java.jsonl'],'java')
    run(['cargo','build','-p','sunrise-protocol','--example','debug_packet_probe'],'compile-rust')
    run([lab.cargo_target()/'debug/examples/debug_packet_probe',build/'java.jsonl',build/'rust.jsonl'],'rust')
    java=[json.loads(x) for x in (build/'java.jsonl').read_text().splitlines()]
    rust=[json.loads(x) for x in (build/'rust.jsonl').read_text().splitlines()]
    assert len(java)==len(rust)==5+1+11+7
    # Encodings: the bytes; the registry's size; refused and accepted decodes: ok with the values, or error.
    for a,b in zip(java,rust):
        key='wire' if a['packet'] in ('sample','request') else 'count' if a['packet']=='registry' else 'result'
        assert a[key]==b[key],(a,b)
    base.ensure_sources_unchanged(sources,tested)
    lab.write_json(build/'report.json',{'pass':True,'cases':len(java),'sources_sha256':tested})
    print(f'Debug packets: {len(java)} native wire encodings match byte for byte')


if __name__=='__main__':main()
