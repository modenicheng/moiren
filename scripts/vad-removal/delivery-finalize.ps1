#requires -Version 5.1
# Durable target-side delivery acknowledgment only. The two independent hashes
# are caller trust anchors; no export/controller CWT is restored, no transfer,
# native admission, reboot or cleanup is performed by this module.
Set-StrictMode -Version 2
. (Join-Path $PSScriptRoot 'closed-export.ps1')

function ConvertFrom-RemovalDeliveryJson([string]$Json){
    if(-not('Moiren.RemovalDeliveryJsonV1.Strict' -as[type])){
        # PS5/PS7 ConvertFrom-Json can silently merge duplicate names. Validate
        # JSON grammar and decoded, case-insensitive property uniqueness first;
        # this is a bounded in-process parser, never an external tool or native API.
        Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.IO;
using System.Text;
namespace Moiren.RemovalDeliveryJsonV1 {
    public sealed class Strict {
        private readonly string text;
        private int offset;
        private int tokens;
        private Strict(string value) { text = value; }
        public static void Validate(string value) {
            if (value == null || value.Length == 0 || value.Length > 16777216)
                throw new InvalidDataException("Bounded nonempty delivery JSON required.");
            Strict parser = new Strict(value);
            parser.Value(0); parser.Space();
            if (parser.offset != value.Length) parser.Fail();
        }
        private void Fail() { throw new InvalidDataException("Strict delivery JSON grammar refused."); }
        private void Space() {
            while (offset < text.Length && (text[offset] == ' ' || text[offset] == '\t' ||
                text[offset] == '\r' || text[offset] == '\n')) offset++;
        }
        private bool Take(char value) {
            Space();
            if (offset < text.Length && text[offset] == value) { offset++; return true; }
            return false;
        }
        private void Need(char value) { if (!Take(value)) Fail(); }
        private void Value(int depth) {
            if (depth > 32 || ++tokens > 100000) throw new InvalidDataException("Delivery JSON structural cap exceeded.");
            Space(); if (offset >= text.Length) { Fail(); return; }
            char c = text[offset];
            if (c == '{') {
                offset++;
                HashSet<string> names = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
                if (Take('}')) return;
                do {
                    Space(); string name = String();
                    if (!names.Add(name)) throw new InvalidDataException("Duplicate JSON property refused: " + name);
                    Need(':'); Value(depth + 1);
                    if (Take('}')) return;
                    Need(',');
                } while (true);
            }
            if (c == '[') {
                offset++;
                if (Take(']')) return;
                do { Value(depth + 1); if (Take(']')) return; Need(','); } while (true);
            }
            if (c == '"') { String(); return; }
            if (c == 't') { Literal("true"); return; }
            if (c == 'f') { Literal("false"); return; }
            if (c == 'n') { Literal("null"); return; }
            Number();
        }
        private void Literal(string value) {
            if (offset + value.Length > text.Length ||
                System.String.CompareOrdinal(text, offset, value, 0, value.Length) != 0) Fail();
            offset += value.Length;
        }
        private int Hex() {
            if (offset + 4 > text.Length) { Fail(); return 0; }
            int value = 0;
            for (int i = 0; i < 4; i++) {
                char c = text[offset++];
                int digit = c >= '0' && c <= '9' ? c - '0' : c >= 'a' && c <= 'f' ? c - 'a' + 10 :
                    c >= 'A' && c <= 'F' ? c - 'A' + 10 : -1;
                if (digit < 0) Fail();
                value = value * 16 + digit;
            }
            return value;
        }
        private string String() {
            Need('"'); StringBuilder result = new StringBuilder();
            while (offset < text.Length) {
                char c = text[offset++];
                if (c == '"') return result.ToString();
                if (c < 32) Fail();
                if (c == '\\') {
                    if (offset >= text.Length) Fail();
                    char escape = text[offset++];
                    switch (escape) {
                        case '"': c = '"'; break; case '\\': c = '\\'; break; case '/': c = '/'; break;
                        case 'b': c = '\b'; break; case 'f': c = '\f'; break; case 'n': c = '\n'; break;
                        case 'r': c = '\r'; break; case 't': c = '\t'; break;
                        case 'u': c = (char)Hex(); break;
                        default: Fail(); break;
                    }
                    if (Char.IsHighSurrogate(c)) {
                        if (offset + 2 > text.Length || text[offset++] != '\\' || text[offset++] != 'u') Fail();
                        char low = (char)Hex(); if (!Char.IsLowSurrogate(low)) Fail();
                        result.Append(c); result.Append(low); continue;
                    }
                    if (Char.IsLowSurrogate(c)) Fail();
                } else if (Char.IsHighSurrogate(c)) {
                    if (offset >= text.Length || !Char.IsLowSurrogate(text[offset])) Fail();
                    result.Append(c); result.Append(text[offset++]); continue;
                } else if (Char.IsLowSurrogate(c)) Fail();
                result.Append(c);
            }
            Fail(); return null;
        }
        private bool Digit() { return offset < text.Length && text[offset] >= '0' && text[offset] <= '9'; }
        private void Number() {
            if (offset < text.Length && text[offset] == '-') offset++;
            if (offset >= text.Length) { Fail(); return; }
            if (text[offset] == '0') offset++;
            else { if (text[offset] < '1' || text[offset] > '9') Fail(); while (Digit()) offset++; }
            if (offset < text.Length && text[offset] == '.') { offset++; if (!Digit()) Fail(); while (Digit()) offset++; }
            if (offset < text.Length && (text[offset] == 'e' || text[offset] == 'E')) {
                offset++; if (offset < text.Length && (text[offset] == '+' || text[offset] == '-')) offset++;
                if (!Digit()) Fail(); while (Digit()) offset++;
            }
        }
    }
}
'@
    }
    [Moiren.RemovalDeliveryJsonV1.Strict]::Validate($Json)
    if(-not$Json.TrimStart().StartsWith('{')){throw 'Delivery JSON root object required; array unrolling is not authority.'}
    return ConvertFrom-Json -InputObject $Json
}

function Assert-RemovalDeliveryObject($Value){
    if($null-eq$Value-or$Value-isnot[pscustomobject]){throw 'Typed delivery JSON object required; no array/scalar coercion.'}
}

function Assert-RemovalDeliveryVersion($Value){
    Assert-RemovalDeliveryObject $Value
    if(($Value.SchemaVersion-isnot[int]-and$Value.SchemaVersion-isnot[long])-or$Value.SchemaVersion-ne1){
        throw 'Exact integer delivery SchemaVersion 1 required.'
    }
}

function Assert-RemovalDeliveryTargetHost([string]$RunDirectory){
    if([Environment]::MachineName-cne'LAPTOP-NP49RG6H'-or
        [Security.Principal.WindowsIdentity]::GetCurrent().User.Value-cne'S-1-5-21-3080296863-1242338503-1346362208-1001'){
        throw 'Wrong delivery target host/SID; no file finalization.'
    }
    if($RunDirectory-cnotmatch'\AD:\\MoirenTest\\'){throw 'Delivery run must belong to the existing target test workspace.'}
}

function Assert-RemovalDeliveryPin($Pin,[long]$MaximumBytes){
    Assert-RemovalDeliveryObject $Pin
    if($null-eq$Pin-or$Pin.Path-isnot[string]-or-not$Pin.Path-or
        ($Pin.Bytes-isnot[int]-and$Pin.Bytes-isnot[long])-or$Pin.Bytes-lt0-or$Pin.Bytes-gt$MaximumBytes-or
        $Pin.Sha256-isnot[string]-or$Pin.Sha256-cnotmatch'\A[A-Fa-f0-9]{64}\z'){
        throw 'Exact typed bounded delivery path/byte/SHA256 pin required.'
    }
    if(@($Pin.PSObject.Properties).Count-ne3){throw 'Delivery pin must contain exactly Path, Bytes and Sha256.'}
    if($Pin.Path-cnotmatch'\A[A-Za-z]:\\'-or$Pin.Path-match'[\x00-\x1f"*?<>|]'-or
        $Pin.Path.Substring(2).Contains(':')-or[IO.Path]::GetFullPath($Pin.Path)-cne$Pin.Path){throw 'Canonical original delivery pin path required.'}
}

function Assert-RemovalDeliverySamePin($Actual,$Expected,[switch]$ContentOnly){
    Assert-RemovalDeliveryPin $Actual 68157440;Assert-RemovalDeliveryPin $Expected 68157440
    if(( -not$ContentOnly-and$Actual.Path-cne$Expected.Path)-or$Actual.Bytes-ne$Expected.Bytes-or$Actual.Sha256-ine$Expected.Sha256){
        throw 'Delivery original file pin/path/content mismatch.'
    }
}

function Read-RemovalDeliveryJson([string]$Path,[long]$MaximumBytes){
    $file=Read-RemovalClosedExportFile $Path -Capture
    if($file.Bytes-eq0-or$file.Bytes-gt$MaximumBytes){throw 'Delivery JSON empty or exceeds byte cap.'}
    $json=[Text.UTF8Encoding]::new($false,$true).GetString($file.Content)
    $value=ConvertFrom-RemovalDeliveryJson $json;Assert-RemovalDeliveryObject $value
    [pscustomobject]@{Pin=[pscustomobject]@{Path=$Path;Bytes=$file.Bytes;Sha256=$file.Sha256};Json=$json;Value=$value}
}

function Assert-RemovalDeliveryFlags($Value,[string]$Meaning,[bool]$Transferred){
    Assert-RemovalDeliveryVersion $Value
    if(($Value.SchemaVersion-isnot[int]-and$Value.SchemaVersion-isnot[long])-or$Value.SchemaVersion-ne1-or
        $Value.Meaning-cne$Meaning-or$Value.Transferred-isnot[bool]-or$Value.Transferred-ne$Transferred-or
        $Value.NativePassed-isnot[bool]-or$Value.NativePassed-or$Value.Qualified-isnot[bool]-or$Value.Qualified){
        throw 'Delivery/local receipt schema or exact nonqualification flags differ.'
    }
}

function Assert-RemovalDeliveryHostPaths($Receipt){
    $directory=$Receipt.HostEvidenceDirectory
    $seen=New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    # Host paths are metadata from the independently pinned host receipt, not
    # target-local paths. Do not probe or reinterpret them on this machine.
    foreach($path in @($directory,$Receipt.HostArchivePin.Path,$Receipt.HostInventoryPin.Path,$Receipt.HostLocalAckPin.Path)){
        if($path-isnot[string]-or$path-cnotmatch'\A[A-Za-z]:\\'-or$path-match'[\x00-\x1f"*?<>|]'-or
            $path.Substring(2).Contains(':')-or[IO.Path]::GetFullPath($path)-cne$path){throw 'Canonical absolute host evidence metadata paths required.'}
    }
    foreach($pair in @(@($Receipt.HostArchivePin,$Receipt.ArchivePin),@($Receipt.HostInventoryPin,$Receipt.InventoryPin),
        @($Receipt.HostLocalAckPin,$Receipt.SourceLocalAckPin))){
        Assert-RemovalDeliverySamePin $pair[0] $pair[1] -ContentOnly
        if([IO.Path]::GetDirectoryName($pair[0].Path)-cne$directory-or-not$seen.Add($pair[0].Path)){
            throw 'Distinct host receipt members must belong to its exact evidence directory.'
        }
    }
}

function Read-RemovalDeliveryProvenance([string]$ArchivePath){
    $stream=[IO.File]::Open($ArchivePath,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    $zip=$null
    try{
        if($stream.Length-gt68157440){throw 'Delivery archive exceeds bounded container cap.'}
        $zip=[IO.Compression.ZipArchive]::new($stream,[IO.Compression.ZipArchiveMode]::Read,$true)
        if($zip.Entries.Count-gt512){throw 'Delivery archive exceeds 512 members.'}
        $names=New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
        foreach($entry in $zip.Entries){
            $name=$entry.FullName;$parts=$name.TrimEnd('/').Split('/')
            $attributes=[long]$entry.ExternalAttributes-band4294967295
            if(-not$names.Add($name)-or$name.Contains('\')-or$name.Contains(':')-or$name.StartsWith('/')-or
                $name-notmatch'\A(run/|export/(provenance|inventory)\.json\z)'-or
                ($attributes-band1024)-ne0-or(($attributes-shr16)-band61440)-eq40960){
                throw 'Duplicate/noncanonical/link ZIP member refused.'
            }
            foreach($part in $parts){if(-not$part-or$part-in@('.','..')-or$part.TrimEnd(' ','.')-cne$part-or
                $part-match'[<>:"|?*\x00-\x1f]'){throw 'Noncanonical ZIP path segment refused.'}}
            if($name.StartsWith('run/')-and$parts.Count-gt9){throw 'ZIP run depth exceeds eight.'}
        }
        $entries=@($zip.Entries|Where-Object{$_.FullName-ceq'export/provenance.json'})
        if($entries.Count-ne1-or$entries[0].Length-le0-or$entries[0].Length-gt16777216){throw 'Unique bounded original archive provenance required.'}
        $input=$entries[0].Open();$copy=New-Object IO.MemoryStream
        try{
            $buffer=New-Object byte[] 65536;$total=[long]0
            while(($read=$input.Read($buffer,0,$buffer.Length))-gt0){
                $total+=$read;if($total-gt$entries[0].Length){throw 'Archive provenance exceeded declared length.'}
                $copy.Write($buffer,0,$read)
            }
            if($total-ne$entries[0].Length){throw 'Archive provenance became short.'}
            $bytes=$copy.ToArray();$json=[Text.UTF8Encoding]::new($false,$true).GetString($bytes)
            $value=ConvertFrom-RemovalDeliveryJson $json;Assert-RemovalDeliveryObject $value
            [pscustomobject]@{Value=$value;Member=[pscustomobject]@{Name='export/provenance.json';Kind='File'
                Bytes=[long]$bytes.Length;Sha256=(Get-RemovalClosedExportHash $bytes)}}
        }finally{$copy.Dispose();$input.Dispose()}
    }finally{if($null-ne$zip){$zip.Dispose()};$stream.Dispose()}
}

function Assert-RemovalDeliveryInventory($Inventory,$RunMembers,$ProvenanceMember){
    Assert-RemovalDeliveryVersion $Inventory;Assert-RemovalDeliveryObject $Inventory.Limits
    if(($Inventory.SchemaVersion-isnot[int]-and$Inventory.SchemaVersion-isnot[long])-or$Inventory.SchemaVersion-ne1-or
        $Inventory.Members-isnot[array]-or$Inventory.Members.Count-lt2-or$Inventory.Members.Count-gt511){
        throw 'Original bounded delivery inventory schema required.'
    }
    foreach($pair in @(@('RunRelativeDepth',8),@('Members',512),@('FileBytes',16777216),@('TotalBytes',67108864))){
        $value=$Inventory.Limits.($pair[0])
        if(($value-isnot[int]-and$value-isnot[long])-or$value-ne$pair[1]){throw 'Original delivery inventory caps differ.'}
    }
    $expected=@{}
    foreach($row in (@($RunMembers)+@($ProvenanceMember))){$expected[$row.Name]=$row}
    if($Inventory.Members.Count-ne$expected.Count){throw 'Inventory/whole target run membership differs.'}
    $seen=New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    foreach($row in $Inventory.Members){
        Assert-RemovalDeliveryObject $row
        if($row.Name-isnot[string]-or-not$seen.Add($row.Name)-or-not$expected.ContainsKey($row.Name)){
            throw 'Duplicate/foreign/noncanonical delivery inventory member.'
        }
        $original=$expected[$row.Name]
        if($row.Name-cne$original.Name-or$row.Kind-cne$original.Kind-or
            ($row.Bytes-isnot[int]-and$row.Bytes-isnot[long])-or$row.Bytes-ne$original.Bytes-or
            ($row.Kind-ceq'Directory'-and$null-ne$row.Sha256)-or
            ($row.Kind-ceq'File'-and($row.Sha256-isnot[string]-or$row.Sha256-ine$original.Sha256))){
            throw 'Original delivery inventory member path/kind/length/hash differs.'
        }
    }
}

function Confirm-RemovalRunDelivery {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)][string]$RunDirectory,
        [Parameter(Mandatory=$true)][string]$DeliveryReceiptPath,
        [Parameter(Mandatory=$true)][ValidatePattern('^[A-Fa-f0-9]{64}$')][string]$ExpectedDeliveryReceiptSha256,
        [Parameter(Mandatory=$true)][ValidatePattern('^[A-Fa-f0-9]{64}$')][string]$ExpectedLocalAckSha256,
        [switch]$UserRebootConfirmed)
    $ErrorActionPreference='Stop'
    if(-not$UserRebootConfirmed){throw 'User reboot confirmation required; delivery finalization never reboots.'}
    Assert-RemovalDeliveryTargetHost $RunDirectory
    Assert-FiniteToolPath $RunDirectory $false;Assert-FiniteToolPath $DeliveryReceiptPath $false
    if(-not[IO.Directory]::Exists($RunDirectory)){throw 'Original closed target run directory required.'}
    $archivePath=$RunDirectory+'.zip';$inventoryPath=$archivePath+'.inventory.json';$localAckPath=$RunDirectory+'.archive-local.json'
    $resultPath=Join-Path $RunDirectory 'controller-result.json';$output=$RunDirectory+'.archive-delivered.json'
    Assert-RemovalClosedExportDestination $output $RunDirectory '.json'
    if($DeliveryReceiptPath-ceq$output-or$DeliveryReceiptPath.StartsWith($RunDirectory+'\',[StringComparison]::OrdinalIgnoreCase)){
        throw 'Host delivery receipt must remain outside immutable original run.'
    }
    # Exclude writers/deletion on the original durable chain through final ack
    # readback. These are ordinary file handles, not restored native ownership.
    $handles=New-Object Collections.Generic.List[object]
    try{
        foreach($path in @($DeliveryReceiptPath,$localAckPath,$inventoryPath,$archivePath,$resultPath)){
            Assert-FiniteToolPath $path $false
            $handles.Add([IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read))
        }
        $delivery=Read-RemovalDeliveryJson $DeliveryReceiptPath 1048576
        if($delivery.Pin.Sha256-ine$ExpectedDeliveryReceiptSha256){throw 'Independent host delivery receipt SHA256 differs.'}
        $receipt=$delivery.Value;Assert-RemovalDeliveryFlags $receipt 'HOST_ARCHIVE_DELIVERY_VERIFIED' $true
        if($receipt.Target-cne'moiren-lan'-or$receipt.RunDirectory-cne$RunDirectory-or
            $receipt.Nonce-isnot[string]-or$receipt.Nonce-cnotmatch'\A[0-9a-f]{32}\z'){
            throw 'Host delivery receipt target/run/nonce differs.'
        }
        $local=Read-RemovalDeliveryJson $localAckPath 1048576
        if($local.Pin.Sha256-ine$ExpectedLocalAckSha256){throw 'Independent original local acknowledgment SHA256 differs.'}
        Assert-RemovalDeliveryFlags $local.Value 'LOCAL_ARCHIVE_VERIFICATION_ONLY' $false
        if($local.Value.Nonce-cne$receipt.Nonce){throw 'Local/host delivery nonce differs.'}
        Assert-RemovalDeliverySamePin $local.Pin $receipt.SourceLocalAckPin
        $inventory=Read-RemovalDeliveryJson $inventoryPath 16777216
        Assert-RemovalDeliverySamePin $inventory.Pin $receipt.InventoryPin
        Assert-RemovalDeliverySamePin $inventory.Pin $local.Value.InventoryPin
        Assert-RemovalDeliverySamePin $receipt.ArchivePin $local.Value.ArchivePin
        if($receipt.ArchivePin.Path-cne$archivePath){throw 'Host receipt archive path differs from fixed target sibling.'}
        $result=Read-RemovalDeliveryJson $resultPath 1048576
        Assert-RemovalDeliverySamePin $result.Pin $receipt.ResultPin
        Assert-RemovalDeliverySamePin $result.Pin $local.Value.ResultPin
        $saved=$result.Value;Assert-RemovalDeliveryVersion $saved
        if($saved.SchemaVersion-ne1-or$saved.ControllerComplete-isnot[bool]-or-not$saved.ControllerComplete-or
            $saved.Nonce-cne$receipt.Nonce-or$saved.Safe-isnot[bool]-or
            $saved.NativePassed-isnot[bool]-or$saved.NativePassed-or$saved.Qualified-isnot[bool]-or$saved.Qualified){
            throw 'Original closed controller result differs; failed safety remains diagnostic evidence.'
        }
        Assert-RemovalDeliveryHostPaths $receipt
        $provenance=Read-RemovalDeliveryProvenance $archivePath;$p=$provenance.Value;Assert-RemovalDeliveryVersion $p
        if($p.SchemaVersion-ne1-or$p.Nonce-cne$receipt.Nonce-or$p.ResultJson-cne$result.Json-or
            $p.Transferred-isnot[bool]-or$p.Transferred-or$p.NativePassed-isnot[bool]-or$p.NativePassed-or
            $p.Qualified-isnot[bool]-or$p.Qualified){throw 'Archived original result/provenance differs.'}
        Assert-RemovalDeliverySamePin $p.ResultPin $result.Pin
        Assert-RemovalDeliverySamePin $p.ManifestPin $receipt.ManifestPin
        Assert-RemovalDeliverySamePin $saved.Manifest $p.ManifestPin
        if($p.ResultJson-isnot[string]-or$p.InputJson-isnot[string]-or$p.CollectorProvenance-isnot[string]-or
            $p.CollectorProvenance-cnotin@('RepositoryCollector','InjectedCollectorSource')-or$p.SourcePins-isnot[array]-or
            $p.StagedScriptPins-isnot[array]-or$p.TerminalPins-isnot[array]-or$p.TerminalPins.Count-gt512-or
            $p.SourcePins.Count-gt64-or$p.StagedScriptPins.Count-notin@(0,5)){throw 'Typed bounded original provenance strings and pin arrays required.'}
        foreach($pin in @($p.TerminalPins)+@($p.StagedScriptPins)){
            Assert-RemovalDeliveryPin $pin 16777216
            if(-not$pin.Path.StartsWith($RunDirectory+'\',[StringComparison]::Ordinal)){throw 'Terminal/staged pin outside original run.'}
        }
        foreach($pin in $p.SourcePins){
            Assert-RemovalDeliveryObject $pin
            if(@($pin.PSObject.Properties).Count-ne4){throw 'Source pin must contain exactly Name, Path, Bytes and Sha256.'}
            $plain=[pscustomobject]@{Path=$pin.Path;Bytes=$pin.Bytes;Sha256=$pin.Sha256}
            Assert-RemovalDeliveryPin $plain 1048576
            if($pin.Name-isnot[string]-or$pin.Name-cnotmatch'\A[A-Za-z0-9][A-Za-z0-9_.-]*\.ps1\z'){
                throw 'Typed original source name required.'
            }
        }
        $inputs=ConvertFrom-RemovalDeliveryJson $p.InputJson;Assert-RemovalDeliveryObject $inputs
        Assert-RemovalDeliverySamePin $inputs.ManifestPin $p.ManifestPin
        $inputSources=@()
        if($null-ne$inputs.PSObject.Properties['Pins']){
            Assert-RemovalDeliveryObject $inputs.Pins
            if($null-ne$inputs.Pins.PSObject.Properties['Sources']){
                if($inputs.Pins.Sources-isnot[array]){throw 'Original input source pins must remain an array.'}
                $inputSources=$inputs.Pins.Sources
            }
        }
        if($inputSources.Count-ne$p.SourcePins.Count){throw 'Archived source pins differ from original input snapshot.'}
        for($i=0;$i-lt$inputSources.Count;$i++){
            $original=$inputSources[$i];$source=$p.SourcePins[$i];Assert-RemovalDeliveryObject $original
            if(@($original.PSObject.Properties).Count-ne4-or$original.Name-cne$source.Name-or$original.Path-cne$source.Path-or
                ($original.Bytes-isnot[int]-and$original.Bytes-isnot[long])-or$original.Bytes-ne$source.Bytes-or
                $original.Sha256-isnot[string]-or$original.Sha256-ine$source.Sha256){
                throw 'Archived source pin content/name differs from original input snapshot.'
            }
        }
        # Reuse only closed-file validation helpers. This durable view never
        # calls the original seal getter nor recreates export/controller CWTs.
        $view=[pscustomobject]@{Directory=$RunDirectory;Nonce=$receipt.Nonce;ResultPin=$result.Pin;ResultJson=$result.Json
            ManifestPin=$p.ManifestPin;StagedScriptPins=@($p.StagedScriptPins);TerminalPins=@($p.TerminalPins)}
        $members=Get-RemovalClosedExportInventory $view
        if($inventory.Value.Nonce-cne$receipt.Nonce){throw 'Original inventory nonce differs.'}
        Assert-RemovalDeliverySamePin $inventory.Value.ResultPin $result.Pin
        Assert-RemovalDeliveryInventory $inventory.Value $members $provenance.Member
        $inventoryMember=[pscustomobject]@{Name='export/inventory.json';Kind='File';Bytes=$inventory.Pin.Bytes;Sha256=$inventory.Pin.Sha256}
        $all=@($members)+@($provenance.Member,$inventoryMember);$total=[long]0
        foreach($member in $all){$total+=$member.Bytes}
        if($all.Count-gt512-or$total-gt67108864){throw 'Complete durable delivery exceeds original bounded archive policy.'}
        $state=[pscustomobject]@{ArchivePath=$archivePath;Members=$all}
        $actualArchive=Test-RemovalClosedExportArchive $state
        Assert-RemovalDeliverySamePin $actualArchive $receipt.ArchivePin
        # Recheck literal run membership/pins after independent archive readback;
        # no stale BEFORE, Explorer, external sources or tools are reopened.
        if((Get-RemovalClosedExportJson (Get-RemovalClosedExportInventory $view))-cne(Get-RemovalClosedExportJson $members)){
            throw 'Original target run changed during durable delivery verification.'
        }
        $value=[ordered]@{SchemaVersion=1;Meaning='TARGET_ARCHIVE_DELIVERY_ACKNOWLEDGED';Target='moiren-lan'
            RunDirectory=$RunDirectory;Nonce=$receipt.Nonce;ResultPin=$result.Pin;ManifestPin=$p.ManifestPin
            SourceLocalAckPin=$local.Pin;ArchivePin=$actualArchive;InventoryPin=$inventory.Pin
            DeliveryReceiptPin=$delivery.Pin;DeliveryReceiptJson=$delivery.Json
            HostEvidenceDirectory=$receipt.HostEvidenceDirectory;HostArchivePin=$receipt.HostArchivePin
            HostInventoryPin=$receipt.HostInventoryPin;HostLocalAckPin=$receipt.HostLocalAckPin
            Transferred=$true;NativePassed=$false;Qualified=$false
            Limitations=@('Acknowledges the independently pinned host delivery receipt and unchanged target bytes.'
                'Host files are asserted by that receipt, not reread on the target.'
                'No native qualification, release, transfer command, reboot or cleanup is authorized.')}
        $bytes=[Text.UTF8Encoding]::new($false,$true).GetBytes((Get-RemovalClosedExportJson $value))
        if($bytes.Length-gt2097152){throw 'Target delivery acknowledgement exceeds 2 MiB cap.'}
        return Write-RemovalClosedExportBytes $output $bytes
    }finally{foreach($handle in $handles){$handle.Dispose()}}
}
